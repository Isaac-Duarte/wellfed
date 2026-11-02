import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

/// How far a colour channel may move before its pixel counts as changed.
///
/// Linux renders the same on x86_64 and arm64 but for rounding in scaled
/// images: measured, every such pixel moved by at most 4 of 255. A real
/// change moves far more: one letter dropped from a label changed 22 pixels,
/// each by more than 32. So a pixel past this counts, and none may.
const int channelTolerance = 8;

/// The usual comparison, except that a pixel counts as changed only when a
/// channel moves by more than [channelTolerance]. Font rasterizers differ
/// by far more than that, so the files still belong to Linux alone.
class LinuxGoldenComparator extends LocalFileComparator {
  LinuxGoldenComparator(super.testFile);

  @override
  Future<bool> compare(Uint8List imageBytes, Uri golden) async {
    final Uint8List expected = Uint8List.fromList(await getGoldenBytes(golden));
    final int? changed = await _changedPixels(imageBytes, expected);
    if (changed == 0) return true;

    final ComparisonResult result = await GoldenFileComparator.compareLists(imageBytes, expected);
    final String detail = changed == null
        ? 'The images differ in size.'
        : '$changed pixels moved by more than $channelTolerance in a channel.';
    throw FlutterError('${await generateFailureOutput(result, golden, basedir)}\n$detail');
  }
}

/// Pixels of `a` and `b` with a channel apart by more than the tolerance;
/// `null` when their sizes differ.
Future<int?> _changedPixels(Uint8List a, Uint8List b) async {
  final _Rgba first = await _decode(a);
  final _Rgba second = await _decode(b);
  if (first.width != second.width || first.height != second.height) return null;

  var changed = 0;
  for (var pixel = 0; pixel < first.bytes.length; pixel += 4) {
    for (var channel = pixel; channel < pixel + 4; channel++) {
      if ((first.bytes[channel] - second.bytes[channel]).abs() > channelTolerance) {
        changed++;
        break;
      }
    }
  }
  return changed;
}

typedef _Rgba = ({int width, int height, Uint8List bytes});

Future<_Rgba> _decode(Uint8List png) async {
  final ui.Codec codec = await ui.instantiateImageCodec(png);
  final ui.Image image = (await codec.getNextFrame()).image;
  final ByteData? data = await image.toByteData(format: ui.ImageByteFormat.rawStraightRgba);
  if (data == null) throw FlutterError('A golden image did not decode.');
  return (width: image.width, height: image.height, bytes: data.buffer.asUint8List());
}
