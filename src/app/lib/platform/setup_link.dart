import 'dart:async';

import 'package:flutter/services.dart' show MethodChannel, MissingPluginException;
import 'package:flutter_reactive_ble/flutter_reactive_ble.dart';

import 'package:wellfed/platform/desktop_bluetooth.dart';
import 'package:wellfed/platform/host.dart';
import 'package:wellfed/src/rust/api/setup.dart';

/// Whether this device's Bluetooth is on, as far as setup can act on it.
enum BluetoothPower {
  /// On, or in a state the owner can't fix by turning it on (no permission,
  /// no radio): the scan says what is wrong. The plugin reports a missing
  /// Nearby devices permission before the adapter's power, so without it a
  /// phone with Bluetooth off reads as on: setup asks for the permission first.
  /// A computer that has not said yet whether its app may use Bluetooth reads
  /// as on too.
  on,

  /// Turned off: the owner can turn it on.
  off,
}

const _bluetooth = MethodChannel('io.github.isaacduarte.wellfed/bluetooth');

/// Whether Bluetooth is on now, by the first word of `power`. A plugin that says
/// nothing in [SetupLink.powerWait] is taken as on: the scan says what is wrong.
/// The answer never waits for the stream's cancel, which a plugin may finish late.
Future<BluetoothPower> bluetoothNow(Stream<BluetoothPower> power) {
  final now = Completer<BluetoothPower>();
  void answer(BluetoothPower power) {
    if (!now.isCompleted) now.complete(power);
  }

  final StreamSubscription<BluetoothPower> reading = power.listen(
    answer,
    onError: (Object _) => answer(BluetoothPower.on),
    onDone: () => answer(BluetoothPower.on),
  );
  return now.future
      .timeout(SetupLink.powerWait, onTimeout: () => BluetoothPower.on)
      .whenComplete(() => unawaited(reading.cancel()));
}

/// Whether Bluetooth is off now or goes off within [SetupLink.powerSettle]: a
/// scan or a link fails a moment before the plugin reports the adapter going
/// down, so a failure read at once would still see it on.
Future<bool> bluetoothOffSoon(Stream<BluetoothPower> power) {
  final off = Completer<bool>();
  void answer(bool answer) {
    if (!off.isCompleted) off.complete(answer);
  }

  final StreamSubscription<BluetoothPower> reading = power.listen(
    (power) {
      if (power == BluetoothPower.off) answer(true);
    },
    onError: (Object _) => answer(false),
    onDone: () => answer(false),
  );
  return off.future
      .timeout(SetupLink.powerSettle, onTimeout: () => false)
      .whenComplete(() => unawaited(reading.cancel()));
}

/// A connected feeder's control characteristic.
abstract interface class BleLink {
  /// Each notification's bytes, in order; the session reassembles frames.
  Stream<List<int>> get notifications;

  /// Write `pieces` in order, each acknowledged before the next.
  Future<void> write(List<List<int>> pieces);

  /// Hang up.
  Future<void> close();
}

/// The radio half of BLE setup: find a feeder in setup mode, connect, and
/// carry the setup session's bytes over its control characteristic. What
/// the bytes mean is the Rust session's business.
///
/// A phone's radio is `flutter_reactive_ble`. The plugin has no desktop
/// side, so a computer's is the bridge's ([DesktopBluetooth]), behind the
/// same static calls, so every port and state machine runs unchanged.
class SetupLink implements BleLink {
  SetupLink._(this._ble, this._connection, this._control, this.notifications);

  /// How long a scan waits for the first feeder.
  static const findTimeout = Duration(seconds: 30);

  /// How long a feeder gets to answer a connection.
  static const connectTimeout = Duration(seconds: 15);

  /// The plugin first says whether Bluetooth is on about 0.5 s after it
  /// starts listening (its own delay), and at once after that. Past this it is
  /// taken as on, and the scan says what is wrong.
  static const powerWait = Duration(seconds: 2);

  /// How long after a failed scan or link the adapter's going down may still
  /// be reported: Android stops the radio's work before it says why.
  static const powerSettle = Duration(seconds: 1);

  final FlutterReactiveBle _ble;
  final StreamSubscription<ConnectionStateUpdate> _connection;
  final QualifiedCharacteristic _control;

  @override
  final Stream<List<int>> notifications;

  /// Every advertisement of the setup service until cancelled; a feeder
  /// shows up again as its signal changes.
  static Stream<DiscoveredDevice> scan() => onDesktopHost ? _desktop.scan() : _phoneScan();

  static const _desktop = DesktopBluetooth();

  static Stream<DiscoveredDevice> _phoneScan() => FlutterReactiveBle().scanForDevices(
    withServices: [Uuid.parse(setupService())],
    scanMode: ScanMode.lowLatency,
    // BLUETOOTH_SCAN is declared neverForLocation (Android 12+).
    requireLocationServicesEnabled: false,
  );

  /// Whether this device's Bluetooth is on, now and each time it changes. The
  /// plugin's "not known yet" is left out.
  static Stream<BluetoothPower> power() => onDesktopHost ? _desktop.power() : _phonePower();

  static Stream<BluetoothPower> _phonePower() => FlutterReactiveBle().statusStream
      .where((status) => status != BleStatus.unknown)
      .map((status) => status == BleStatus.poweredOff ? BluetoothPower.off : BluetoothPower.on);

  /// Ask Android to turn Bluetooth on: its own dialog, or Bluetooth settings
  /// where it can't show one. A computer shows its Bluetooth settings where
  /// it can. [power] says when it is on.
  static Future<void> askToTurnOn() async {
    if (onDesktopHost) return _desktop.openSettings();
    try {
      await _bluetooth.invokeMethod<void>('turnOn');
    } on MissingPluginException {
      // An Android build without the app's own channel has nothing to ask.
    }
  }

  /// Connect to `device` and ask for notifications. The plugin does not
  /// say when the subscription lands, so a first write may arrive before
  /// it; the feeder holds the message until the link opens and answers
  /// then.
  static Future<BleLink> open(DiscoveredDevice device) => onDesktopHost ? _desktop.open(device) : _openPhone(device);

  static Future<SetupLink> _openPhone(DiscoveredDevice device) async {
    final ble = FlutterReactiveBle();
    final connected = Completer<void>();
    final StreamSubscription<ConnectionStateUpdate> connection = ble
        .connectToDevice(
          id: device.id,
          servicesWithCharacteristicsToDiscover: {
            Uuid.parse(setupService()): [Uuid.parse(setupControl())],
          },
          connectionTimeout: connectTimeout,
        )
        .listen(
          (update) {
            if (update.connectionState == DeviceConnectionState.connected && !connected.isCompleted) {
              connected.complete();
            }
            if (update.connectionState == DeviceConnectionState.disconnected && !connected.isCompleted) {
              connected.completeError(update.failure?.message ?? 'the feeder did not connect');
            }
          },
          onError: (Object error) {
            if (!connected.isCompleted) connected.completeError(error);
          },
        );
    try {
      await connected.future.timeout(connectTimeout);
      // Writes never depend on it (the pieces fit the smallest MTU), but a
      // larger one carries notifications in fewer packets.
      await ble.requestMtu(deviceId: device.id, mtu: 247);
    } catch (_) {
      await connection.cancel();
      rethrow;
    }

    final control = QualifiedCharacteristic(
      serviceId: Uuid.parse(setupService()),
      characteristicId: Uuid.parse(setupControl()),
      deviceId: device.id,
    );
    final Stream<List<int>> notifications = ble.subscribeToCharacteristic(control).asBroadcastStream();
    return SetupLink._(ble, connection, control, notifications);
  }

  @override
  Future<void> write(List<List<int>> pieces) async {
    for (final piece in pieces) {
      await _ble.writeCharacteristicWithResponse(_control, value: piece).timeout(connectTimeout);
    }
  }

  @override
  Future<void> close() => _connection.cancel();
}
