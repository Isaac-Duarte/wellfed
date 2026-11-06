import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/state/restore/restore.dart';

import 'restore_support.dart';

void main() {
  test('a device that may not restore is refused before anything is read or run', () async {
    final ports = FakeRestorePorts()..restoresFeeder = false;
    final RestoreFlow flow = restoreFlow(ports);
    addTearDown(flow.dispose);

    await flow.load();
    flow.setUnderstood(true);
    await flow.start();

    expect(flow.state, isA<RestoreUnavailable>());
    expect(ports.held, isEmpty);
    expect(ports.specs, isEmpty);
    expect(ports.runs, isEmpty);
  });

  testWidgets('the screen says the computer cannot restore, and offers no way on', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    try {
      final ports = FakeRestorePorts()..restoresFeeder = false;
      await restoreOpen(tester, ports);

      expect(
        find.textContaining('This computer cannot restore the original firmware.', findRichText: true),
        findsOneWidget,
      );
      expect(find.text('I understand'), findsNothing);
      expect(find.text('Restore original firmware'), findsOneWidget);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });
}
