import 'dart:async';

import 'package:flutter/services.dart' show MethodChannel, MissingPluginException;
import 'package:flutter_reactive_ble/flutter_reactive_ble.dart';

import 'package:wellfed/src/rust/api/setup.dart';

/// Whether this phone's Bluetooth is on, as far as setup can act on it.
enum BluetoothPower {
  /// On, or in a state the owner can't fix by turning it on (no permission,
  /// no radio): the scan says what is wrong.
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
class SetupLink implements BleLink {
  SetupLink._(this._ble, this._connection, this._control, this.notifications);

  /// How long a scan waits for the first feeder.
  static const findTimeout = Duration(seconds: 30);

  /// How long a feeder gets to answer a connection.
  static const connectTimeout = Duration(seconds: 15);

  /// The plugin says whether Bluetooth is on within milliseconds of being
  /// asked. Past this it is taken as on, and the scan says what is wrong.
  static const powerWait = Duration(seconds: 2);

  final FlutterReactiveBle _ble;
  final StreamSubscription<ConnectionStateUpdate> _connection;
  final QualifiedCharacteristic _control;

  @override
  final Stream<List<int>> notifications;

  /// Every advertisement of the setup service until cancelled; a feeder
  /// shows up again as its signal changes.
  static Stream<DiscoveredDevice> scan() => FlutterReactiveBle().scanForDevices(
    withServices: [Uuid.parse(setupService())],
    scanMode: ScanMode.lowLatency,
    // BLUETOOTH_SCAN is declared neverForLocation (Android 12+).
    requireLocationServicesEnabled: false,
  );

  /// Whether this phone's Bluetooth is on, now and each time it changes. The
  /// plugin's "not known yet" is left out.
  static Stream<BluetoothPower> power() => FlutterReactiveBle().statusStream
      .where((status) => status != BleStatus.unknown)
      .map((status) => status == BleStatus.poweredOff ? BluetoothPower.off : BluetoothPower.on);

  /// Ask Android to turn Bluetooth on: its own dialog, or Bluetooth settings
  /// where it can't show one. [power] says when it is on.
  static Future<void> askToTurnOn() async {
    try {
      await _bluetooth.invokeMethod<void>('turnOn');
    } on MissingPluginException {
      // A desktop build has no Android to ask; its Bluetooth is the system's.
    }
  }

  /// Connect to `device` and ask for notifications. The plugin does not
  /// say when the subscription lands, so a first write may arrive before
  /// it; the feeder holds the message until the link opens and answers
  /// then.
  static Future<SetupLink> open(DiscoveredDevice device) async {
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
