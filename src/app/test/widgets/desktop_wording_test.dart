import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/meal_call.dart';
import 'package:wellfed/screens/setup/name_step.dart';
import 'package:wellfed/state/feeder/call_recording.dart';

import '../support/fakes.dart';

/// `body` with the platform set to `platform`, for the test alone.
Future<void> on(TargetPlatform platform, Future<void> Function() body) async {
  debugDefaultTargetPlatformOverride = platform;
  try {
    await body();
  } finally {
    debugDefaultTargetPlatformOverride = null;
  }
}

void main() {
  group('A refused microphone says where to allow it', () {
    for (final (TargetPlatform platform, String where) in [
      (TargetPlatform.android, "Android's settings"),
      (TargetPlatform.macOS, 'System Settings › Privacy & Security › Microphone'),
      (TargetPlatform.linux, "this computer's sound settings"),
    ]) {
      test(
        'on ${platform.name}',
        () => on(platform, () async {
          expect(recordingWords(RecordingProblem.denied), contains(where));
        }),
      );
    }
  });

  testWidgets('naming on a desktop says the key stays on this computer', (tester) async {
    await on(TargetPlatform.macOS, () async {
      await tester.pumpWidget(
        themed(NameStep(initial: 'Mac', action: 'Continue', suggest: () async => null, onDone: (_, _) async => null)),
      );
      expect(find.textContaining('stays on this computer', findRichText: true), findsOneWidget);
      expect(find.textContaining('phone', findRichText: true), findsNothing);
    });
  });
}
