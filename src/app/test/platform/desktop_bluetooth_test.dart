import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_reactive_ble/flutter_reactive_ble.dart' show DiscoveredDevice;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/desktop_bluetooth.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/screens/setup/pairing_view.dart';
import 'package:wellfed/src/rust/api/bluetooth.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/state/pairing/pairing.dart';

import '../support/fakes.dart';

/// A bridge handle the test drives: its stream is the test's, and a stop
/// ends it, as the bridge's does within a round.
class _FakeHandle<T> {
  // A cancel with no onCancel answers in the root zone, which a widget test's
  // fake clock never runs.
  final events = StreamController<T>(onCancel: () async {});
  int stops = 0;
  int disposals = 0;

  void stopped() {
    stops++;
    unawaited(events.close());
  }
}

class _FakeScan extends _FakeHandle<ScanEvent> implements SetupScan {
  @override
  Stream<ScanEvent> run() => events.stream;

  @override
  void stop() => stopped();

  @override
  void dispose() => disposals++;

  @override
  bool get isDisposed => disposals > 0;
}

class _FakeWatch extends _FakeHandle<AdapterPower> implements PowerWatch {
  @override
  Stream<AdapterPower> run() => events.stream;

  @override
  void stop() => stopped();

  @override
  void dispose() => disposals++;

  @override
  bool get isDisposed => disposals > 0;
}

/// A connected feeder through the bridge: what was written, and the
/// notifications the test sends.
class _FakeLink with FakeOpaque implements BluetoothLink {
  // ignore: close_sinks, the test ends with it
  final notified = StreamController<Uint8List>();
  final written = <List<int>>[];
  int closes = 0;
  BridgeError? failWrites;

  @override
  Stream<Uint8List> notifications() => notified.stream;

  @override
  Future<void> write({required List<int> piece}) async {
    if (failWrites case final error?) throw error;
    written.add(piece);
  }

  @override
  Future<void> close() async => closes++;
}

const _broken = BridgeError(kind: ErrorKind.bluetooth, message: 'the feeder dropped the link');

HeardFeeder _heard(String id, {int rssi = -60}) => HeardFeeder(id: id, name: 'Wellfed', rssi: rssi);

/// Pairing's ports with the desktop radio in place of the phone's plugin.
class _DesktopPairingPorts extends FakePairingPorts {
  _DesktopPairingPorts(this.radio);

  final DesktopBluetooth radio;

  @override
  Stream<DiscoveredDevice> scan() => radio.scan();

  @override
  Future<BleLink> open(DiscoveredDevice device) {
    opened.add(device);
    return radio.open(device);
  }
}

void main() {
  group('scan', () {
    test('each feeder heard is a device, and a cancel stops the scan', () async {
      final handle = _FakeScan();
      final radio = DesktopBluetooth(newScan: () => handle);
      final heard = <DiscoveredDevice>[];

      final StreamSubscription<DiscoveredDevice> scanning = radio.scan().listen(heard.add);
      handle.events.add(ScanEvent.heard(feeder: _heard('hci0/dev_00_11', rssi: -48)));
      await pumpEventQueue();

      expect(heard.single.id, 'hci0/dev_00_11');
      expect(heard.single.rssi, -48);

      await scanning.cancel();
      expect(handle.stops, 1);
      expect(handle.disposals, 1);
    });

    test("a scan that fails ends with a radio failure, not the feeder's answer", () async {
      final handle = _FakeScan();
      final radio = DesktopBluetooth(newScan: () => handle);
      final errors = <Object>[];
      final done = Completer<void>();

      radio.scan().listen((_) {}, onError: errors.add, onDone: done.complete);
      handle.events.add(const ScanEvent.failed(error: _broken));
      await handle.events.close();
      await done.future;

      expect(errors.single, isA<BluetoothFailure>());
      expect(handle.disposals, 1, reason: 'a stream that ended by itself frees its handle');
    });
  });

  group('power', () {
    test('off and on come through as the plugin says them', () async {
      final handle = _FakeWatch();
      final radio = DesktopBluetooth(newWatch: () => handle);
      final said = <BluetoothPower>[];

      final StreamSubscription<BluetoothPower> watching = radio.power().listen(said.add);
      handle.events
        ..add(AdapterPower.poweredOff)
        ..add(AdapterPower.poweredOn);
      await pumpEventQueue();
      await watching.cancel();

      expect(said, [BluetoothPower.off, BluetoothPower.on]);
      expect(handle.stops, 1);
    });

    test('a radio that says nothing reads as on, and the scan says what is wrong', () async {
      final handle = _FakeWatch();
      final radio = DesktopBluetooth(newWatch: () => handle);

      final Future<BluetoothPower> now = bluetoothNow(radio.power());
      await handle.events.close();

      expect(await now, BluetoothPower.on);
    });
  });

  group('link', () {
    test('pieces go in order, notifications come back, and a close hangs up once', () async {
      final link = _FakeLink();
      final radio = DesktopBluetooth(openLink: (id) async => link);

      final BleLink opened = await radio.open(feederNearby('feeder-1'));
      final heard = <List<int>>[];
      opened.notifications.listen(heard.add);
      await opened.write([
        [1, 2],
        [3],
      ]);
      link.notified.add(Uint8List.fromList([9, 8]));
      await pumpEventQueue();
      await opened.close();
      await opened.close();

      expect(link.written, [
        [1, 2],
        [3],
      ]);
      expect(heard, [
        [9, 8],
      ]);
      expect(link.closes, 1);
    });

    test("a failed connection or write is the radio's failure", () async {
      final link = _FakeLink()..failWrites = _broken;
      final refused = DesktopBluetooth(openLink: (id) async => throw _broken);
      final radio = DesktopBluetooth(openLink: (id) async => link);

      await expectLater(refused.open(feederNearby('feeder-1')), throwsA(isA<BluetoothFailure>()));
      final BleLink opened = await radio.open(feederNearby('feeder-1'));
      await expectLater(
        opened.write([
          [1],
        ]),
        throwsA(isA<BluetoothFailure>()),
      );
    });
  });

  testWidgets("a desktop link that breaks mid-setup says to move closer, as a phone's does", (tester) async {
    tester.view.physicalSize = const Size(1080, 3000);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    final handle = _FakeScan();
    final link = _FakeLink()..failWrites = _broken;
    final ports = _DesktopPairingPorts(DesktopBluetooth(newScan: () => handle, openLink: (id) async => link));
    final flow = PairingFlow(FakeBook(), ports);
    await tester.pumpWidget(themed(PairingView(flow: flow, ble: true, onRetry: () {}, onNewCode: () {})));

    unawaited(flow.runSetup(wifi: null, name: 'Kitchen'));
    await tester.pump();
    handle.events.add(ScanEvent.heard(feeder: _heard('feeder-1')));
    await settle(tester);
    expect(flow.state, isA<PairingFinding>());
    if (flow.state case PairingFinding(:final found)) flow.choose(found.values.single);
    await tester.pump();
    await tester.pump(SetupLink.powerSettle);
    await settle(tester);

    expect(ports.opened.single.id, 'feeder-1');
    expect(handle.stops, 1, reason: 'the scan stops once a feeder is picked');
    expect(find.text('The Bluetooth link dropped'), findsOneWidget);
    flow.dispose();
    await tester.pumpWidget(const SizedBox());
  });
}
