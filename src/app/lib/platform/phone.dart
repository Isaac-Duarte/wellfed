import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter/services.dart';
import 'package:network_info_plus/network_info_plus.dart';
import 'package:permission_handler/permission_handler.dart';

/// What setup asks of the phone itself. Behind this, so its screens are tested
/// with no Android.
abstract interface class PhonePorts {
  /// Everything setup uses is already granted.
  Future<bool> setupGranted();

  /// Ask for what setup uses; whether Bluetooth, which it can't go without, came.
  Future<bool> askForSetup();

  /// Ask for Bluetooth alone, as each pairing does.
  Future<bool> askForBluetooth();

  /// Open this app's page in Android settings.
  Future<void> openSettings();

  /// The Wi-Fi this phone is on, when Android will say.
  Future<String?> currentNetwork();

  /// The phone's model, for a name.
  Future<String?> model();
}

/// [PhonePorts] on Android.
class AppPhonePorts implements PhonePorts {
  /// The phone this runs on.
  const AppPhonePorts();

  /// Bluetooth to reach the feeder, and location only because Android needs it
  /// to read the Wi-Fi's name.
  static const List<Permission> _setup = [
    Permission.bluetoothScan,
    Permission.bluetoothConnect,
    Permission.locationWhenInUse,
  ];

  static const List<Permission> _bluetooth = [Permission.bluetoothScan, Permission.bluetoothConnect];

  @override
  Future<bool> setupGranted() async {
    for (final Permission permission in _setup) {
      if (!await permission.isGranted) return false;
    }
    return true;
  }

  @override
  Future<bool> askForSetup() async {
    final Map<Permission, PermissionStatus> granted = await _setup.request();
    return _bluetooth.every((permission) => granted[permission]?.isGranted ?? false);
  }

  @override
  Future<bool> askForBluetooth() async {
    final Map<Permission, PermissionStatus> granted = await _bluetooth.request();
    return granted.values.every((status) => status.isGranted);
  }

  @override
  Future<void> openSettings() async {
    await openAppSettings();
  }

  /// The plugins only answer on a phone; typing the name always works.
  @override
  Future<String?> currentNetwork() async {
    try {
      if (!await Permission.locationWhenInUse.isGranted) return null;
      return await NetworkInfo().getWifiName();
    } on MissingPluginException {
      return null;
    }
  }

  @override
  Future<String?> model() async {
    try {
      return (await DeviceInfoPlugin().androidInfo).model;
    } on MissingPluginException {
      return null;
    }
  }
}
