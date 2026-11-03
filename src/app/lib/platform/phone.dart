import 'dart:io';

import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter/services.dart';
import 'package:network_info_plus/network_info_plus.dart';
import 'package:permission_handler/permission_handler.dart';
import 'package:wellfed/state/wifi/networks.dart';

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

  /// The networks this phone hears, for the feeder's Wi-Fi.
  Future<NetworkScan> scanNetworks();

  /// The phone's model, for a name.
  Future<String?> model();
}

/// [PhonePorts] on Android and on the Linux desktop.
///
/// Linux has no runtime permissions: BlueZ and NetworkManager answer any
/// process of the logged-in user, so everything setup uses is granted.
class AppPhonePorts implements PhonePorts {
  /// The device this runs on.
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
    if (Platform.isLinux) return true;
    for (final Permission permission in _setup) {
      if (!await permission.isGranted) return false;
    }
    return true;
  }

  @override
  Future<bool> askForSetup() async {
    if (Platform.isLinux) return true;
    final Map<Permission, PermissionStatus> granted = await _setup.request();
    return _bluetooth.every((permission) => granted[permission]?.isGranted ?? false);
  }

  @override
  Future<bool> askForBluetooth() async {
    if (Platform.isLinux) return true;
    final Map<Permission, PermissionStatus> granted = await _bluetooth.request();
    return granted.values.every((status) => status.isGranted);
  }

  /// Never asked on Linux, where nothing is ever refused.
  @override
  Future<void> openSettings() async {
    if (Platform.isLinux) return;
    await openAppSettings();
  }

  /// The plugins only answer on a phone, or a Linux desktop running
  /// NetworkManager; typing the name always works.
  @override
  Future<String?> currentNetwork() async {
    try {
      if (!Platform.isLinux && !await Permission.locationWhenInUse.isGranted) return null;
      return await NetworkInfo().getWifiName();
    } on MissingPluginException {
      return null;
    }
  }

  /// On Linux the hostname, which tells one computer from another where the
  /// distribution's name would not.
  @override
  Future<NetworkScan> scanNetworks() => scanNearbyNetworks();

  @override
  Future<String?> model() async {
    if (Platform.isLinux) return Platform.localHostname;
    try {
      return (await DeviceInfoPlugin().androidInfo).model;
    } on MissingPluginException {
      return null;
    }
  }
}

const _wifi = MethodChannel('io.github.isaacduarte.wellfed/wifi');

/// The networks this phone hears, asking for location first if it isn't
/// allowed: Android lists no networks without it. Never throws: what can't be
/// listed says why, and the owner types the name instead.
Future<NetworkScan> scanNearbyNetworks() async {
  try {
    if (!await Permission.locationWhenInUse.isGranted && !(await Permission.locationWhenInUse.request()).isGranted) {
      return const NetworksUnread(ScanProblem.permission);
    }
    final Map<String, Object?>? answer = await _wifi.invokeMapMethod<String, Object?>('scanNetworks');
    return _scanFrom(answer);
  } on MissingPluginException {
    return const NetworksUnread(ScanProblem.unavailable);
  }
}

/// MainActivity's answer: `problem`, or `networks` as maps.
NetworkScan _scanFrom(Map<String, Object?>? answer) {
  final Object? problem = answer?['problem'];
  if (problem is String) {
    return NetworksUnread(ScanProblem.values.asNameMap()[problem] ?? ScanProblem.unavailable);
  }
  final Object? networks = answer?['networks'];
  if (networks is! List<Object?>) return const NetworksUnread(ScanProblem.unavailable);
  return NetworksSeen(
    List.unmodifiable([
      for (final network in networks)
        if (network case {
          'ssid': final String ssid,
          'frequencyMhz': final int frequencyMhz,
          'level': final int level,
          'secured': final bool secured,
        })
          SeenNetwork(ssid: ssid, frequencyMhz: frequencyMhz, level: level, secured: secured),
    ]),
  );
}
