import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:wellfed/platform/recorder.dart';

/// Why a recording came to nothing.
enum RecordingProblem {
  /// The microphone is not allowed.
  denied,

  /// The microphone failed, or gave nothing.
  failed,

  /// The app left the foreground, where Android records only silence.
  interrupted,
}

/// The meal call recorded on this phone. It stops itself at [most], about
/// the longest clip the feeder keeps, and hands what it recorded to `done`,
/// as stopping it does.
class CallRecording extends ChangeNotifier {
  /// A recording from `_recorder`, handed to `_done` once it stops.
  CallRecording(this._recorder, this._done) {
    _lifecycle = AppLifecycleListener(onPause: _paused);
  }

  /// The feeder keeps about 8 s once the silence at either end is trimmed.
  static const most = Duration(seconds: 8);

  /// The count the screen shows moves once a second.
  static const _tickEvery = Duration(seconds: 1);

  final Recorder _recorder;
  final void Function(Uint8List wav) _done;

  Timer? _tick;
  bool _disposed = false;

  /// Between a tap and the microphone answering: a second tap must not
  /// start a second recording, or a second timer.
  bool _starting = false;

  late final AppLifecycleListener _lifecycle;

  /// Recording now.
  bool get recording => _tick != null;

  /// Whole seconds recorded so far.
  int get seconds => _seconds;
  int _seconds = 0;

  /// Why the last recording came to nothing, until the next starts.
  RecordingProblem? get problem => _problem;
  RecordingProblem? _problem;

  /// Recording now, or about to.
  bool get busy => recording || _starting;

  /// Start recording, unless already.
  Future<void> start() async {
    if (busy) return;
    _starting = true;
    _problem = null;
    notifyListeners();

    final bool started;
    try {
      started = await _recorder.start();
    } on Exception {
      _starting = false;
      _end(RecordingProblem.failed);
      return;
    }
    _starting = false;
    if (_disposed) {
      // The screen went while the microphone was opening: close it, and
      // keep nothing.
      if (started) unawaited(_discard());
      return;
    }
    if (!started) {
      _end(RecordingProblem.denied);
      return;
    }

    _seconds = 0;
    _tick = Timer.periodic(_tickEvery, (_) {
      _seconds++;
      if (_seconds >= most.inSeconds) {
        unawaited(stop());
      } else {
        notifyListeners();
      }
    });
    notifyListeners();
  }

  /// Stop, and hand what was recorded to `done`.
  Future<void> stop() async {
    final Uint8List? wav = await _finish();
    if (wav == null || _disposed) return;
    _done(wav);
  }

  /// Stop, and throw the recording away.
  Future<void> cancel() => _finish();

  /// The app left the foreground: what follows would be silence, so the
  /// recording ends here and nothing is sent.
  void _paused() {
    if (!recording) return;
    unawaited(_finish().then((_) => _end(RecordingProblem.interrupted)));
  }

  Future<void> _discard() async {
    try {
      await _recorder.stop();
    } on Exception {
      // Closing is all that was wanted; a failure here leaves nothing to say.
    }
  }

  /// Stop the microphone: what it recorded, if anything.
  Future<Uint8List?> _finish() async {
    if (!recording) return null;
    _tick?.cancel();
    _tick = null;

    try {
      final Uint8List? wav = await _recorder.stop();
      if (wav == null || wav.isEmpty) {
        _end(RecordingProblem.failed);
        return null;
      }
      _end(null);
      return wav;
    } on Exception {
      _end(RecordingProblem.failed);
      return null;
    }
  }

  void _end(RecordingProblem? problem) {
    _problem = problem;
    if (!_disposed) notifyListeners();
  }

  /// Leaving the screen stops the microphone and keeps nothing.
  @override
  void dispose() {
    _disposed = true;
    _lifecycle.dispose();
    unawaited(_finish());
    super.dispose();
  }
}
