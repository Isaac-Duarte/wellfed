import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/updates.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/update.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/update/update_state.dart';

import '../support/fakes.dart';
import '../support/harness.dart';

void main() {
  test('a device that may not update holds and sends nothing, even when asked to start', () async {
    final book = FakeBook()..feeders = [kitchen];
    final ports = FakeFeederPorts()
      ..updatesFeeder = false
      ..image = const ImageCheck.ready(provisioning: ImageProvisioning.consumer, build: 'cd', version: 7);
    final feeder = FeederController(book, ports, kitchen);
    addTearDown(feeder.dispose);

    await feeder.update.start(const UpdateOffer_Newer(build: 'cd', version: 7));

    expect(feeder.update.state, isA<UpdateUnavailable>());
    expect(ports.holds, 0);
    expect(ports.pushes, isEmpty);
  });

  testWidgets('the firmware screen says the computer cannot update', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    try {
      final book = FakeBook()..feeders = [kitchen];
      final ports = FakeFeederPorts()..updatesFeeder = false;
      final feeder = FeederController(book, ports, kitchen);
      addTearDown(feeder.dispose);

      await tester.pumpWidget(themed(UpdatesScreen(feeder: feeder)));
      await tester.pump();

      expect(
        find.textContaining("This computer cannot update a feeder's firmware.", findRichText: true),
        findsOneWidget,
      );
      expect(find.text('Firmware'), findsOneWidget);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });
}
