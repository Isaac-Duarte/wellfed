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

  /// How long a picked recording waits for the session to come back. It covers
  /// the session's longest retry and a connect: the picker can sit open long
  /// enough for the session to end behind it.
  static const reachWait = Duration(seconds: 45);

  final FeederController feeder;
  final PickRecording pick;

  @override
  State<MealCallScreen> createState() => _MealCallScreenState();
}

class _MealCallScreenState extends State<MealCallScreen> {
  CallInfo? _call;
  bool _reading = true;
  bool _busy = false;

  /// A session is being waited for, not a write.
  bool _reaching = false;

  /// Why the last recording came to nothing, until the next is chosen. A
  /// snackbar alone can pass while the screen is still coming back.
  String? _problem;

  @override
  void initState() {
    super.initState();
    unawaited(_read());
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

  /// A write's answer, then the feeder's view again.
  Future<void> _write(FeederRequest request) async {
    setState(() => _busy = true);
    final reply = await widget.feeder.ask(request);
    if (!mounted) return;
    setState(() => _busy = false);
    if (reply is FeederReply_Call) {
      setState(() => _call = reply.call);
    } else {
      showProblem(context, reply);
      unawaited(_read());
    }
  }

  /// An upload left unknown shows here as what the feeder holds, and is not
  /// sent again.
  Future<void> _upload() async {
    setState(() => _problem = null);
    final Uint8List? wav;
    try {
      wav = await widget.pick();
    } on Exception {
      if (mounted) {
        setState(() => _problem = notAskedWords(NotAsked.badRecording));
      }
      return;
    }
    if (wav == null || !mounted) return;

    setState(() {
      _busy = true;
      _reaching = true;
    });
    final reply = await _sendOnce(wav);
    if (!mounted) return;
    setState(() {
      _busy = false;
      _reaching = false;
    });

    if (reply == null) {
      setState(
        () =>
            _problem = "Couldn't reach the feeder. Choose the recording again.",
      );
    } else if (reply is FeederReply_Call) {
      setState(() => _call = reply.call);
    } else {
      setState(() => _problem = replyProblem(reply));
      unawaited(_read());
    }
  }

  /// Send `wav` once the session answers; `null` if it does not in time. A
  /// session that died behind the picker can still look open, so a read goes
  /// first: it is free to get lost, and the upload then goes on a session seen
  /// alive. This wait belongs to this screen only. A feed's path never waits,
  /// and an upload that may have started is never sent again.
  Future<FeederReply?> _sendOnce(Uint8List wav) async {
    const step = Duration(milliseconds: 500);
    final feeder = widget.feeder;
    for (
      var waited = Duration.zero;
      waited < MealCallScreen.reachWait;
      waited += step
    ) {
      if (feeder.connection == Connection.online) {
        final read = await feeder.ask(const FeederRequest.getCall());
        if (!mounted) return null;
        if (read is! FeederReply_NotSent && read is! FeederReply_Unanswered) {
          setState(() {
            _reaching = false;
            if (read is FeederReply_Call) _call = read.call;
          });

          final reply = await feeder.ask(FeederRequest.uploadCall(wav: wav));
          // Only "not sent" means none of it went out, so only that waits.
          if (reply is! FeederReply_NotSent) return reply;
          if (!mounted) return null;
          setState(() => _reaching = true);
        }
      }
      await Future<void>.delayed(step);
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
            onPressed: _busy ? null : _upload,
            child: Text(
              stored ? 'Replace the recording' : 'Choose a recording',
            ),
          ),
          if (stored) ...[
            const SizedBox(height: Space.s2),
            OutlinedButton(
              style: OutlinedButton.styleFrom(foregroundColor: c.fault),
              onPressed: _busy
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
