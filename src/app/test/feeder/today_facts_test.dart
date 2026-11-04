import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';
import 'feeder_support.dart';

void main() {
  testWidgets('only the facts that need the owner show by themselves', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);
    session.status(fakeCard(adapter: false, batteryPercent: 15, rssi: -80));
    await settle(tester);

    expect(find.text('Batteries 15%'), findsOneWidget);
    expect(find.text('Weak'), findsOneWidget);
    expect(find.text('Change in 9 days'), findsNothing);
    expect(find.text('Everything else looks fine'), findsOneWidget);
  });

  testWidgets('no meal links to the schedule', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);
    session.status(fakeCard());
    await settle(tester);

    expect(find.text('No meals today'), findsOneWidget);
    await feederTap(tester, find.text('Add a meal'));
    expect(DefaultTabController.of(tester.element(find.byType(TabBar))).index, scheduleTab);
  });
}
