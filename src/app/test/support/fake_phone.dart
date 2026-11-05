import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/phone.dart';
import 'package:wellfed/platform/recorder.dart';
import 'package:wellfed/state/wifi/networks.dart';

/// A phone with everything granted, and no plugins to ask.
class FakePhonePorts implements PhonePorts {
  @override
  Future<bool> setupGranted() async => true;

  @override
  Future<bool> askForSetup() async => true;

  @override
  Future<bool> askForBluetooth() async => true;

  @override
  Future<void> openSettings() async {}

  /// The Wi-Fi this phone is on.
  String? network;

  /// What a scan finds: by default, the phone lists nothing, as with no plugin.
  NetworkScan scan = const NetworksUnread(ScanProblem.unavailable);

  @override
  Future<String?> currentNetwork() async => network;

  @override
  Future<NetworkScan> scanNetworks() async => scan;

  @override
  Future<String?> model() async => null;
}

/// The microphone: starts when [allowed], and stops with [recorded].
class FakeRecorder implements Recorder {
  bool allowed = true;
  Uint8List? recorded = Uint8List.fromList([1, 2, 3]);
  int starts = 0;
  int stops = 0;

  /// Holds `start`'s answer until completed, as a slow microphone would.
  Completer<void>? opening;

  @override
  Future<bool> start() async {
    starts++;
    await opening?.future;
    return allowed;
  }

  @override
  Future<Uint8List?> stop() async {
    stops++;
    return recorded;
  }
}
