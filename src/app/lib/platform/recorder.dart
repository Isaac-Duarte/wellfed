import 'dart:io';
import 'dart:typed_data';

import 'package:path_provider/path_provider.dart';
import 'package:record/record.dart';

/// The phone's microphone, for the meal call.
abstract interface class Recorder {
  /// Start recording; false when the microphone is not allowed. Asks for the
  /// permission the first time.
  Future<bool> start();

  /// Stop: the WAV recorded, or null when nothing was.
  Future<Uint8List?> stop();
}

/// [Recorder] on Android, through `record`.
///
/// A WAV at 16 kHz, mono: the bridge converts any WAV to the feeder's 8 kHz
/// stream, and twice that rate keeps its low-pass well clear of the voice.
class AppRecorder implements Recorder {
  final _recorder = AudioRecorder();

  @override
  Future<bool> start() async {
    if (!await _recorder.hasPermission()) return false;

    // One left by an app that was killed mid-recording goes now.
    final File file = await _file();
    if (file.existsSync()) await file.delete();

    await _recorder.start(
      const RecordConfig(encoder: AudioEncoder.wav, sampleRate: 16000, numChannels: 1),
      path: file.path,
    );
    return true;
  }

  @override
  Future<Uint8List?> stop() async {
    final String? path = await _recorder.stop();
    if (path == null) return null;

    // The recording is the owner's voice: it stays on the phone only as long
    // as the upload needs it, whether or not it could be read.
    final file = File(path);
    try {
      return await file.readAsBytes();
    } finally {
      if (file.existsSync()) await file.delete();
    }
  }

  Future<File> _file() async => File('${(await getTemporaryDirectory()).path}/meal-call.wav');
}
