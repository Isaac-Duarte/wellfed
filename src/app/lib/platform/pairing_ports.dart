import 'dart:async';

import 'package:flutter_reactive_ble/flutter_reactive_ble.dart' show DiscoveredDevice;

import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/setup.dart';
import 'package:wellfed/src/rust/api/store.dart';

/// What a pairing reaches outside Dart: the Rust session, the radio and NSD.
/// Behind this, so its screens are tested with no Rust and no radio.
abstract interface class PairingPorts {
  /// A setup session under `label`, giving the feeder `wifi` if there is any.
  SetupDriver newDriver(String label, WifiCredentials? wifi);

  /// A recovery session under `label`.
  Recovery newRecovery(String label);

  /// Every advertisement of the setup service until cancelled.
  Stream<DiscoveredDevice> scan();

  /// Connect to `device`.
  Future<BleLink> open(DiscoveredDevice device);

  /// Whether this phone's Bluetooth is on, now and as it changes.
  Stream<BluetoothPower> bluetooth();

  /// Ask Android to turn Bluetooth on.
  Future<void> turnOnBluetooth();

  /// Where the feeder with `identity` (hex) answers, within `wait`.
  Future<String?> locate(String identity, Duration wait);

  /// A key as the store and NSD's `id` spell it.
  String hexOf(List<int> identity);

  /// Whether the feeder `feeder` at `address` took this phone, by opening a
  /// session with it, which sends it nothing.
  Future<SetupSettled> settle(List<int> feeder, String address);

  /// Each feeder answering on this network, until cancelled.
  Stream<FoundFeeder> browse();
}

/// [PairingPorts] on Android, with this phone's key.
class AppPairingPorts implements PairingPorts {
  /// Ports signed by `_key`.
  const AppPairingPorts(this._key);

  final ClientKey _key;

  @override
  SetupDriver newDriver(String label, WifiCredentials? wifi) => SetupDriver(key: _key, label: label, wifi: wifi);

  @override
  Recovery newRecovery(String label) => Recovery(key: _key, label: label);

  @override
  Stream<DiscoveredDevice> scan() => SetupLink.scan();

  @override
  Future<SetupSettled> settle(List<int> feeder, String address) =>
      settleSetup(key: _key, feeder: feeder, address: address);

  @override
  Future<BleLink> open(DiscoveredDevice device) => SetupLink.open(device);

  @override
  Stream<BluetoothPower> bluetooth() => SetupLink.power();

  @override
  Future<void> turnOnBluetooth() => SetupLink.askToTurnOn();

  @override
  Future<String?> locate(String identity, Duration wait) => Discovery.locate(identity, wait);

  @override
  String hexOf(List<int> identity) => identityHex(identity: identity);

  @override
  Stream<FoundFeeder> browse() => Discovery.browse();
}
