import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'feeder_support.dart';

void main() {
  Future<(FakeBook, FakeFeederPorts, FeederController)> settings(WidgetTester tester, {String? manual}) async {
    final book = FakeBook()
      ..feeders = [
        KnownFeeder(
          identity: feederKitchen.identity,
          name: feederKitchen.name,
          address: feederKitchen.address,
          manualAddress: manual,
        ),
      ];
    final ports = FakeFeederPorts();
    final fleet = FeederFleet(book, ports);
    addTearDown(fleet.dispose);
    await tester.pump();
    ports.session.opened();
    await tester.pump();

    final FeederController feeder = feederOnly(fleet);
    await showTab(tester, SettingsTab(feeder: feeder));
    return (book, ports, feeder);
  }

  Future<void> type(WidgetTester tester, String address) async {
    await tapIn(tester, find.text('Address'));
    await tester.enterText(find.byType(TextField), address);
    await tapIn(tester, find.widgetWithText(FilledButton, 'Save'));
  }

  testWidgets("a typed address is dialed without NSD, and NSD's address stays as it was", (tester) async {
    final (FakeBook book, FakeFeederPorts ports, _) = await settings(tester, manual: '100.64.0.5:4747');

    expect(ports.dialed, ['100.64.0.5:4747']);
    expect(ports.looks, 0);
    // A session at the typed address never stands in for where NSD found it,
    // which updates and restores dial.
    expect(book.answered, isEmpty);
    expect(book.feeders.single.address, feederKitchen.address);
    expect(find.text('100.64.0.5:4747'), findsOneWidget);
  });

  testWidgets('a new address ends the session and dials it at once, port and all', (tester) async {
    final (FakeBook book, FakeFeederPorts ports, _) = await settings(tester);
    expect(find.text('Found on your network'), findsOneWidget);
    final FakeSession first = ports.session;

    await type(tester, ' 100.64.0.5 ');
    expect(book.feeders.single.manualAddress, '100.64.0.5:4747');
    expect(first.closes, 1);
    expect(find.text('100.64.0.5:4747'), findsOneWidget);

    // The session ends as the bridge's does once closed; the next dials with
    // no backoff.
    await first.updates.close();
    await tester.pump();
    expect(ports.dialed, ['lan:4747', '100.64.0.5:4747']);
  });

  testWidgets('a blank address goes back to finding the feeder on the network', (tester) async {
    final (FakeBook book, FakeFeederPorts ports, _) = await settings(tester, manual: '100.64.0.5:4747');

    await type(tester, '');
    expect(book.feeders.single.manualAddress, isNull);
    expect(find.text('Found on your network'), findsOneWidget);

    await ports.session.updates.close();
    await tester.pump();
    expect(ports.dialed.last, 'lan:4747');
  });

  testWidgets('text that is not an address says so and changes nothing', (tester) async {
    final (FakeBook book, FakeFeederPorts ports, _) = await settings(tester);

    await type(tester, 'feeder.local');
    expect(find.textContaining("That isn't an IP address"), findsOneWidget);
    expect(book.feeders.single.manualAddress, isNull);
    expect(ports.session.closes, 0);
  });

  testWidgets('an address that cannot be saved says so and keeps the session', (tester) async {
    final (FakeBook book, FakeFeederPorts ports, _) = await settings(tester);
    book.saveFails = const BridgeError(kind: ErrorKind.store, message: 'read-only');

    await type(tester, '100.64.0.5');
    expect(find.text("Couldn't save the address. Try again."), findsOneWidget);
    expect(find.text('Found on your network'), findsOneWidget);
    expect(ports.session.closes, 0);
  });
}
