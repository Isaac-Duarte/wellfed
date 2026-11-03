import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/clock/clock.dart';
import 'package:wellfed/state/feeder/call_recording.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/replies.dart';

part 'meal_call_view.dart';

/// The meal call's clip: what the feeder holds, and one big button to record
/// a new one. Rust converts
/// the recording and sends it, and the feeder keeps one clip; a new one
/// erases the old as soon as it starts.
class MealCallScreen extends StatefulWidget {
  const MealCallScreen({super.key, required this.feeder, required this.pick});

  /// How long this screen waits for the session to come back, in all: the picker
  /// can sit open long enough for it to end behind it. A reconnect can take
  /// about 38 s: the longest retry (15 s), a look on the network (8 s), a connect
  /// (5 s) and the feeder's welcome (10 s).
  static const reachWait = Duration(seconds: 60);

  final FeederController feeder;
  final PickRecording pick;

  @override
  State<MealCallScreen> createState() => _MealCallScreenState();
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

  /// The feeder is being asked to play the clip.
  bool _playing = false;

  /// A recording made on this phone, sent as a picked file is once it stops.
  late final CallRecording _recording = CallRecording(widget.feeder.recorder, (wav) => unawaited(_record(wav)))
    ..addListener(_recorded);

  void _recorded() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    _recording.dispose();
    super.dispose();
  }

  /// Why the last write came to nothing, until the next. A snackbar alone can
  /// pass while the screen is still coming back.
  String? _problem;

  @override
  void initState() {
    super.initState();
    final Future<FeederReply>? sending = _sending[widget.feeder];
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
    final FeederReply reply = await widget.feeder.ask(const FeederRequest.getCall());
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
    final deadline = Deadline(MealCallScreen.reachWait);
    final FeederReply? read = await _answeredRead(deadline);
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
    final FeederReply reply = await widget.feeder.ask(request);
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

  /// Hear the clip on the feeder. A play changes nothing stored, so nothing
  /// is read back: the answer only says why it did not play, if it did not.
  Future<void> _play() async {
    setState(() {
      _playing = true;
      _problem = null;
    });
    final FeederReply reply = await widget.feeder.ask(const FeederRequest.playCall());
    if (!mounted) return;
    setState(() {
      _playing = false;
      _problem = replyProblem(reply);
    });
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

    final (Uint8List? picked, bool readable) = await widget.feeder.readRecording(widget.pick);
    if (!mounted) return;
    final bool unreadable = !readable;
    setState(() {
      _picking = false;
      if (unreadable) _problem = notAskedWords(NotAsked.badRecording);
      if (picked != null) {
        _busy = true;
        _reaching = true;
      }
    });
    if (picked == null) return;

    await _send(picked, "Couldn't reach the feeder. Choose the recording again.");
  }

  /// A recording made here has stopped: it goes as a picked one does.
  Future<void> _record(Uint8List wav) async {
    if (!mounted || _busy) return;
    setState(() {
      _busy = true;
      _reaching = true;
      _problem = null;
    });
    await _send(wav, "Couldn't reach the feeder. Record it again.");
  }

  /// Send `wav` once, and say what the feeder holds after; `unreached` if
  /// the session never answered.
  Future<void> _send(Uint8List wav, String unreached) async {
    final FeederReply? reply = await _sendOnce(wav);
    if (!mounted) return;
    if (reply == null) {
      setState(() {
        _busy = false;
        _reaching = false;
        _problem = unreached;
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
    final FeederController feeder = widget.feeder;
    final deadline = Deadline(MealCallScreen.reachWait);
    try {
      while (true) {
        final FeederReply? read = await _answeredRead(deadline);
        if (read == null || !mounted) return null;
        setState(() {
          _reaching = false;
          if (read is FeederReply_Call) _call = read.call;
        });

        final Future<FeederReply> sending = feeder.ask(FeederRequest.uploadCall(wav: wav));
        _sending[feeder] = sending;
        final FeederReply reply = await sending;
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
  Future<FeederReply?> _answeredRead(Deadline deadline) async {
    final FeederController feeder = widget.feeder;
    while (!deadline.over) {
      if (feeder.state.connection == Connection.online) {
        final FeederReply? read = await Future.any<FeederReply?>([
          feeder.ask(const FeederRequest.getCall()),
          deadline.done.then((_) => null),
        ]);
        if (read == null || !mounted) return null;
        if (read is! FeederReply_NotSent && read is! FeederReply_Unanswered) {
          return read;
        }
      }
      await deadline.beat();
      if (!mounted) return null;
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final bool stored = _call?.clip is CallClip_Stored;
    // Not while recording: a delete would leave the new clip nowhere to go,
    // and a play would record the feeder's own speaker into it.
    final bool offerStored = stored && !_recording.busy;
    final bool free = !(_busy || _picking || _recording.busy);
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
          style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
        ),
      ),
      body: ListView(
        padding: const EdgeInsets.all(Space.s4),
        children: [
          _ClipCard(
            call: _call,
            reading: _reading,
            offerPlay: offerStored,
            onPlay: _busy || _picking || _playing ? null : () => unawaited(_play()),
          ),
          if (_busy) ...[
            const SizedBox(height: Space.s3),
            Notice(
              tone: Tone.neutral,
              text: _reaching ? 'Reaching the feeder… keep this screen open.' : 'Working… keep this screen open.',
            ),
          ],
          if (_problem case final problem?) ...[
            const SizedBox(height: Space.s3),
            Notice(tone: Tone.warning, text: problem),
          ],
          if (_recording.problem case final problem?) ...[
            const SizedBox(height: Space.s3),
            Notice(tone: Tone.warning, text: _recordingWords(problem)),
          ],
          const SizedBox(height: Space.s6),
          if (_recording.recording) ...[
            _Recording(seconds: _recording.seconds, onStop: () => unawaited(_recording.stop())),
            const SizedBox(height: Space.s2),
            Center(
              child: TextButton(onPressed: () => unawaited(_recording.cancel()), child: const Text('Cancel')),
            ),
          ] else ...[
            _RecordButton(onRecord: free ? () => unawaited(_recording.start()) : null),
            const SizedBox(height: Space.s2),
            Center(
              child: TextButton(
                onPressed: free ? _upload : null,
                child: Text(stored ? 'Replace with a file' : 'Use an audio file instead'),
              ),
            ),
          ],
          const SizedBox(height: Space.s5),
          Text(
            'Plays before each meal that has a meal call, when sounds are on. It never delays a meal.',
            textAlign: TextAlign.center,
            style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: c.textMuted),
          ),
          if (offerStored) ...[
            const SizedBox(height: Space.s4),
            Center(
              child: TextButton(
                style: TextButton.styleFrom(foregroundColor: c.fault),
                onPressed: _busy || _picking ? null : () => unawaited(_write(const FeederRequest.deleteCall())),
                child: const Text('Delete the recording'),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

String _recordingWords(RecordingProblem problem) => switch (problem) {
  RecordingProblem.denied => "The app may not use the microphone. Allow it in Android's settings.",
  RecordingProblem.failed => "Couldn't record. Try again.",
  RecordingProblem.interrupted => 'The recording stopped when you left the app, so nothing was sent.',
};
