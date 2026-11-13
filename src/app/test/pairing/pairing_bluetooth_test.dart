import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_reactive_ble/flutter_reactive_ble.dart' show DiscoveredDevice;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/screens/setup/pairing_view.dart';
import 'package:wellfed/screens/setup/setup_screen.dart';
import 'package:wellfed/state/pairing/pairing.dart';
import 'package:wellfed/state/setup/setup.dart';

import '../support/fakes.dart';

void _tall(WidgetTester tester) {
  tester.view.physicalSize = const Size(1080, 3000);
  tester.view.devicePixelRatio = 3;
  addTearDown(tester.view.reset);
}

/// Setup through to the search, with this phone's Bluetooth as `ports` has it.
Future<void> _toSearch(WidgetTester tester, FakePairingPorts ports) async {
  _tall(tester);
  await tester.pumpWidget(themed(SetupScreen(newFlow: () => SetupFlow(FakeBook(), ports, FakePhonePorts()))));
  await tester.tap(find.text('The Wi-Fi light is blinking'));
  await settle(tester);
  await tester.enterText(find.byType(TextField).at(0), 'Maple St');
  await tester.enterText(find.byType(TextField).at(1), 'correcthorse');
  await tester.pump();
  await tester.tap(find.text('Continue'));
  await settle(tester);
  await tester.tap(find.text('Find the feeder'));
  await settle(tester);
}

void main() {
  group('Bluetooth off', () {
    testWidgets('setup says so before it looks, offers to turn it on, and goes on once it is', (tester) async {
      final ports = FakePairingPorts()..power = BluetoothPower.off;
      await _toSearch(tester, ports);

      expect(find.text('Bluetooth is off'), findsOneWidget);
      expect(find.text('The Bluetooth link dropped'), findsNothing);
      expect(find.text('Looking for feeders in setup mode'), findsNothing);

      await tester.tap(find.text('Turn on Bluetooth'));
      await settle(tester);
      expect(ports.turnOnAsks, 1);

      ports.setPower(BluetoothPower.on);
      await settle(tester);
      expect(find.text('Looking for feeders in setup mode'), findsOneWidget);
      ports.scans.add(feederNearby('feeder-1'));
      await settle(tester);
      expect(find.text('In setup mode · close by'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('a scan that stops as Bluetooth goes off waits for it, and looks again', (tester) async {
      final ports = FakePairingPorts();
      await _toSearch(tester, ports);
      expect(find.text('Looking for feeders in setup mode'), findsOneWidget);

      ports.power = BluetoothPower.off;
      ports.scans.addError(Exception('Bluetooth disabled'));
      await settle(tester);
      expect(find.text('Bluetooth is off'), findsOneWidget);
      expect(find.text('The Bluetooth link dropped'), findsNothing);

      ports.scans = StreamController<DiscoveredDevice>(onCancel: () async {});
      ports.setPower(BluetoothPower.on);
      await settle(tester);
      expect(find.text('Looking for feeders in setup mode'), findsOneWidget);
      ports.scans.add(feederNearby('feeder-1'));
      await settle(tester);
      expect(find.text('In setup mode · close by'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('a link lost with Bluetooth is said so, and a new try waits for it', (tester) async {
      _tall(tester);
      final ports = FakePairingPorts();
      final flow = PairingFlow(FakeBook(), ports);
      await tester.pumpWidget(themed(PairingView(flow: flow, ble: true, onRetry: () {}, onNewCode: () {})));
      unawaited(
        flow.runSetup(
          wifi: const WifiCredentials(ssid: 'Maple St', passphrase: 'x'),
          name: 'Kitchen',
          device: feederNearby('feeder-1'),
        ),
      );
      await tester.pump();
      await tester.pump();

      ports.power = BluetoothPower.off;
      ports.link.bytes.addError(Exception('disconnected'));
      await settle(tester);
      expect(
        flow.state,
        isA<PairingFailed>().having((failed) => failed.failure, 'failure', PairingFailure.bluetoothOff),
      );
      expect(find.text("This phone's Bluetooth went off"), findsOneWidget);
      expect(find.text('Try again with a new code'), findsOneWidget);

      final again = PairingFlow(FakeBook(), ports);
      await tester.pumpWidget(themed(PairingView(flow: again, ble: true, onRetry: () {}, onNewCode: () {})));
      unawaited(again.runSetup(wifi: null, name: 'Kitchen', device: feederNearby('feeder-1')));
      await settle(tester);
      expect(again.state, isA<PairingBluetoothOff>());
      expect(ports.opened, hasLength(1), reason: 'nothing is connected while Bluetooth is off');

      ports.setPower(BluetoothPower.on);
      await settle(tester);
      expect(ports.opened, hasLength(2));
      flow.dispose();
      again.dispose();
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('a link that drops with Bluetooth on still says to move closer', (tester) async {
      _tall(tester);
      final ports = FakePairingPorts();
      final flow = PairingFlow(FakeBook(), ports);
      await tester.pumpWidget(themed(PairingView(flow: flow, ble: true, onRetry: () {}, onNewCode: () {})));
      unawaited(flow.runSetup(wifi: null, name: 'Kitchen', device: feederNearby('feeder-1')));
      await tester.pump();
      await tester.pump();

      ports.link.bytes.addError(Exception('disconnected'));
      await tester.pump(SetupLink.powerSettle);
      await settle(tester);
      expect(find.text('The Bluetooth link dropped'), findsOneWidget);
      flow.dispose();
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('a screen that closes while Bluetooth is off ends the wait', (tester) async {
      final ports = FakePairingPorts()..power = BluetoothPower.off;
      final flow = PairingFlow(FakeBook(), ports);
      var ended = false;
      unawaited(flow.runSetup(wifi: null, name: 'Kitchen').then((_) => ended = true));
      await tester.pump();
      expect(flow.state, isA<PairingBluetoothOff>());

      flow.dispose();
      await tester.pump();
      expect(ended, isTrue);
      expect(ports.opened, isEmpty);
    });

    testWidgets('a plugin that says nothing at first is waited for: off a moment later is off', (tester) async {
      final ports = FakePairingPorts()..power = null;
      final flow = PairingFlow(FakeBook(), ports);
      unawaited(flow.runSetup(wifi: null, name: 'Kitchen'));
      await tester.pump(const Duration(milliseconds: 500));
      expect(flow.state, isA<PairingFinding>(), reason: 'nothing shown as off before the plugin says');
      expect(ports.opened, isEmpty);

      ports.setPower(BluetoothPower.off);
      await tester.pump();
      expect(flow.state, isA<PairingBluetoothOff>());
      flow.dispose();
      await tester.pump();
    });

    testWidgets('a plugin that never says is taken as on, and the search goes on', (tester) async {
      final ports = FakePairingPorts()..power = null;
      final flow = PairingFlow(FakeBook(), ports);
      unawaited(flow.runSetup(wifi: null, name: 'Kitchen'));
      await tester.pump(SetupLink.powerWait);
      await tester.pump();
      expect(flow.state, isA<PairingFinding>());
      ports.scans.add(feederNearby('feeder-1'));
      await tester.pump();
      expect((flow.state as PairingFinding).found, hasLength(1));
      flow.dispose();
      await tester.pump();
    });

    testWidgets('a screen that closes while the plugin answers leaves nothing listening', (tester) async {
      final ports = FakePairingPorts()..power = null;
      final flow = PairingFlow(FakeBook(), ports);
      var ended = false;
      unawaited(flow.runSetup(wifi: null, name: 'Kitchen').then((_) => ended = true));
      await tester.pump();
      flow.dispose();
      ports.setPower(BluetoothPower.off);
      await tester.pump(SetupLink.powerWait);
      expect(ended, isTrue);
      expect(ports.powerListeners, 0);
      expect(flow.state, isNot(isA<PairingBluetoothOff>()));
    });

    testWidgets('a drop the plugin explains a moment later is Bluetooth off, not distance', (tester) async {
      _tall(tester);
      final ports = FakePairingPorts();
      final flow = PairingFlow(FakeBook(), ports);
      await tester.pumpWidget(themed(PairingView(flow: flow, ble: true, onRetry: () {}, onNewCode: () {})));
      unawaited(flow.runSetup(wifi: null, name: 'Kitchen', device: feederNearby('feeder-1')));
      await tester.pump();
      await tester.pump();

      ports.link.bytes.addError(Exception('disconnected'));
      await tester.pump(const Duration(milliseconds: 300));
      ports.setPower(BluetoothPower.off);
      await tester.pump(SetupLink.powerSettle);
      await settle(tester);
      expect(find.text("This phone's Bluetooth went off"), findsOneWidget);
      flow.dispose();
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('a search that finds nothing with Bluetooth off waits for it, not "no feeder"', (tester) async {
      final ports = FakePairingPorts();
      final flow = PairingFlow(FakeBook(), ports);
      unawaited(flow.runSetup(wifi: null, name: 'Kitchen'));
      await tester.pump();
      expect(flow.state, isA<PairingFinding>());

      ports.setPower(BluetoothPower.off);
      await tester.pump(SetupLink.findTimeout);
      await tester.pump();
      expect(flow.state, isA<PairingBluetoothOff>());
      expect(flow.state, isNot(isA<PairingFailed>()));
      flow.dispose();
      await tester.pump();
    });
  });
}
