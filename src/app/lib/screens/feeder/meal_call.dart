import 'dart:async';
import 'dart:typed_data';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';

import '../../src/rust/api/feeder.dart';
import '../../src/rust/api/model.dart';
import '../../state/feeder.dart';
import '../../theme/theme.dart';
import '../../widgets/icons.dart';
import '../../widgets/layout.dart';
import '../../widgets/replies.dart';

/// Picking a recording off the phone, behind this so tests need no platform.
typedef PickRecording = Future<Uint8List?> Function();

Future<Uint8List?> _pickWav() async {
  const wav = XTypeGroup(label: 'WAV', extensions: ['wav']);
  final picked = await openFile(acceptedTypeGroups: const [wav]);
  return picked?.readAsBytes();
}

/// The meal call's clip: a plain screen, as no board draws one. Rust converts
/// the recording and sends it, and the feeder keeps one clip; a new one
/// erases the old as soon as it starts.
class MealCallScreen extends StatefulWidget {
  const MealCallScreen({super.key, required this.feeder, this.pick = _pickWav});

  /// How long this screen waits for the session to come back, in all: the picker
  /// can sit open long enough for it to end behind it. A reconnect can take
  /// about 53 s: the longest retry (30 s), a look on the network (8 s), a connect
  /// (5 s) and the feeder's welcome (10 s).
  static const reachWait = Duration(seconds: 60);

  final FeederController feeder;
  final PickRecording pick;

  @override
  State<MealCallScreen> createState() => _MealCallScreenState();
}

/// When the wait for a session ends. A timer, not a count of pauses: one read
/// on a dead session can take as long as the session takes to notice.
class _Deadline {
  _Deadline(Duration wait) {
    _timer = Timer(wait, () {
      over = true;
      _done.complete();
    });
  }

  late final Timer _timer;
  final _done = Completer<void>();
  bool over = false;

  Future<void> get done => _done.future;

  void cancel() => _timer.cancel();
}

class _MealCallScreenState extends State<MealCallScreen> {
  /// A feeder's upload on its way: it runs on after this screen is left, and a
  /// screen opened meanwhile must not start another.
  static final _sending = Expando<Future<FeederReply>>('meal call upload');

  CallInfo? _call;
  bool _reading = true;
  bool _busy = false;

  /// The picker is open: a second tap would open another.
  bool _picking = false;

  /// A session is being waited for, not a write.
  bool _reaching = false;

  /// Why the last write came to nothing, until the next. A snackbar alone can
  /// pass while the screen is still coming back.
  String? _problem;

  @override
  void initState() {
    super.initState();
    final sending = _sending[widget.feeder];
    if (sending == null) {
      unawaited(_read());
    } else {
      _busy = true;
      unawaited(sending.then(_uploadEnded));
    }
  }

  /// The upload an earlier screen started has ended. Whatever came of it, this
  /// screen reads: unlike [_settle], even after "not sent", as it opened with
  /// nothing read yet.
  Future<void> _uploadEnded(FeederReply reply) async {
    if (!mounted) return;
    setState(() {
      _problem = replyProblem(reply);
      if (reply is FeederReply_Unanswered) _call = null;
    });
    await _readBack();
  }

  Future<void> _read() async {
    final reply = await widget.feeder.ask(const FeederRequest.getCall());
    if (!mounted) return;
    setState(() {
      _reading = false;
      _call = reply is FeederReply_Call ? reply.call : _call;
    });
    showProblem(context, reply);
  }

  /// What the feeder holds after a write, waiting for the session if it went
  /// with the write. Its being gone is said already, and says nothing of what
  /// the write did, so no "not sent" from this read is shown.
  Future<void> _readBack() async {
    if (!mounted) return;
    setState(() {
      _busy = true;
      _reaching = true;
      _reading = true;
    });
    final deadline = _Deadline(MealCallScreen.reachWait);
    final read = await _answeredRead(deadline);
    deadline.cancel();
    if (!mounted) return;

    setState(() {
      _busy = false;
      _reaching = false;
      _reading = false;
      if (read is FeederReply_Call) _call = read.call;
    });
    if (read != null) showProblem(context, read);
  }

  /// A write's answer, then the feeder's view again. One left unknown may have
  /// changed the clip, so the old view goes until the feeder says.
  Future<void> _write(FeederRequest request) async {
    setState(() {
      _busy = true;
      _problem = null;
    });
    final reply = await widget.feeder.ask(request);
    if (!mounted) return;
    await _settle(reply);
  }

  /// A write has ended: say how, and what the feeder holds now.
  Future<void> _settle(FeederReply reply) async {
    setState(() {
      _busy = false;
      _reaching = false;
      if (reply is FeederReply_Call) _call = reply.call;
      if (reply is FeederReply_Unanswered) _call = null;
      _problem = replyProblem(reply);
    });
    // Not sent changed nothing, and its session has gone: nothing to read.
    if (reply is! FeederReply_Call && reply is! FeederReply_NotSent) {
      await _readBack();
    }
  }

  /// An upload left unknown shows here as what the feeder holds, and is not
  /// sent again.
  Future<void> _upload() async {
    // Before the picker, as a second tap can land before the next frame.
    if (_picking || _busy) return;
    setState(() {
      _picking = true;
      _problem = null;
    });

    Uint8List? wav;
    var unreadable = false;
    try {
      wav = await widget.pick();
    } on Exception {
      unreadable = true;
    }
    if (!mounted) return;
    final picked = wav;
    setState(() {
      _picking = false;
      if (unreadable) _problem = notAskedWords(NotAsked.badRecording);
      if (picked != null) {
        _busy = true;
        _reaching = true;
      }
    });
    if (picked == null) return;

    final reply = await _sendOnce(picked);
    if (!mounted) return;
    if (reply == null) {
      setState(() {
        _busy = false;
        _reaching = false;
        _problem = "Couldn't reach the feeder. Choose the recording again.";
      });
    } else {
      await _settle(reply);
    }
  }

  /// Send `wav` once the session answers; `null` if it does not in time. A
  /// session that died behind the picker can still look open, so a read goes
  /// first: it is free to get lost, and the upload then goes on a session seen
  /// alive. The deadline covers the wait and never the upload. This wait
  /// belongs to this screen only. A feed's path never waits, and an upload that
  /// may have started is never sent again.
  Future<FeederReply?> _sendOnce(Uint8List wav) async {
    final feeder = widget.feeder;
    final deadline = _Deadline(MealCallScreen.reachWait);
    try {
      while (true) {
        final read = await _answeredRead(deadline);
        if (read == null || !mounted) return null;
        setState(() {
          _reaching = false;
          if (read is FeederReply_Call) _call = read.call;
        });

        final sending = feeder.ask(FeederRequest.uploadCall(wav: wav));
        _sending[feeder] = sending;
        final reply = await sending;
        _sending[feeder] = null;

        // Only "not sent" means none of it went out, so only that waits.
        if (reply is! FeederReply_NotSent) return reply;
        if (!mounted) return null;
        setState(() => _reaching = true);
      }
    } finally {
      deadline.cancel();
    }
  }

  /// The first read the feeder answers, waiting out a lost session; `null` once
  /// `deadline` passes or the screen goes.
  Future<FeederReply?> _answeredRead(_Deadline deadline) async {
    final feeder = widget.feeder;
    while (!deadline.over) {
      if (feeder.connection == Connection.online) {
        final read = await Future.any<FeederReply?>([
          feeder.ask(const FeederRequest.getCall()),
          deadline.done.then((_) => null),
        ]);
        if (read == null || !mounted) return null;
        if (read is! FeederReply_NotSent && read is! FeederReply_Unanswered) {
          return read;
        }
      }
      await Future<void>.delayed(const Duration(milliseconds: 500));
      if (!mounted) return null;
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final (words, stored) = switch (_call?.clip) {
      CallClip_Stored() => (
        'Stored: ${((_call?.ms ?? 0) / 1000).toStringAsFixed(1)} s a play',
        true,
      ),
      CallClip_Receiving() => ('An upload did not finish', false),
      CallClip_None() => ('No recording stored', false),
      null => (_reading ? 'Reading…' : "Can't read it right now", false),
    };
    return Scaffold(
      appBar: AppBar(
        toolbarHeight: 64,
        automaticallyImplyLeading: false,
        titleSpacing: 4,
        leading: IconButton(
          tooltip: 'Back',
          onPressed: () => Navigator.pop(context),
          icon: StrokeIcon(Glyph.back, color: c.text),
        ),
        title: Text(
          'Meal call',
          style: TextStyle(
            fontSize: 20,
            fontWeight: Fonts.semibold,
            color: c.text,
          ),
        ),
      ),
      body: ListView(
        padding: const EdgeInsets.all(Space.s4),
        children: [
          Text(
            words,
            style: TextStyle(
              fontSize: Sizes.md,
              fontWeight: Fonts.semibold,
              color: c.text,
            ),
          ),
          const SizedBox(height: Space.s2),
          Text(
            'A WAV recording of about 8 seconds at most, which a meal plays '
            'before the food drops. It only ever plays when sound is on, and '
            'never delays a meal.',
            style: TextStyle(
              fontSize: Sizes.base,
              height: 1.5,
              color: c.textMuted,
            ),
          ),
          const SizedBox(height: Space.s4),
          if (_busy) ...[
            Notice(
              tone: Tone.neutral,
              text: _reaching
                  ? 'Reaching the feeder… keep this screen open.'
                  : 'Working… keep this screen open.',
            ),
            const SizedBox(height: Space.s3),
          ],
          if (_problem case final problem?) ...[
            Notice(tone: Tone.warning, text: problem),
            const SizedBox(height: Space.s3),
          ],
          FilledButton(
            onPressed: _busy || _picking ? null : _upload,
            child: Text(
              stored ? 'Replace the recording' : 'Choose a recording',
            ),
          ),
          if (stored) ...[
            const SizedBox(height: Space.s2),
            OutlinedButton(
              style: OutlinedButton.styleFrom(foregroundColor: c.fault),
              onPressed: _busy || _picking
                  ? null
                  : () => unawaited(_write(const FeederRequest.deleteCall())),
              child: const Text('Delete the recording'),
            ),
          ],
        ],
      ),
    );
  }
}
