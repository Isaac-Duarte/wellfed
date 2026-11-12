import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_reactive_ble/flutter_reactive_ble.dart' show DiscoveredDevice;

import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/platform/pairing_ports.dart';
import 'package:wellfed/platform/phone.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/setup.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/pairing/pairing.dart';
import 'package:wellfed/state/wifi/networks.dart';

export 'package:wellfed/platform/discovery.dart' show FoundFeeder;
export 'package:wellfed/src/rust/api/setup.dart' show WifiCredentials;

/// Save `label` as this phone's name; why it failed, or null.
Future<String?> saveLabel(FeederBook book, String label) async {
  try {
    await book.setLabel(label);
    return null;
  } on BridgeError catch (error) {
    return error.message;
  }
}

/// How BLE setup stands.
sealed class SetupState {
  /// A state of setup.
  const SetupState();
}

/// What setup will ask Android for, before it asks.
final class SetupPermissions extends SetupState {
  /// The permissions step.
  const SetupPermissions();
}

/// How to put the feeder in setup mode.
final class SetupMode extends SetupState {
  /// The first step.
  const SetupMode();
}

/// The network the feeder joins.
final class SetupWifi extends SetupState {
  /// The Wi-Fi step, with what was given before.
  const SetupWifi({required this.wifi});

  /// What was given before.
  final WifiCredentials? wifi;
}

/// What the feeder and this phone are called.
final class SetupNames extends SetupState {
  /// The name step.
  const SetupNames({required this.feederName});

  /// What this phone calls the feeder so far.
  final String feederName;
}

/// The pairing itself.
final class SetupPairing extends SetupState {
  /// A pairing under way.
  const SetupPairing({required this.pairing});

  /// Its controller.
  final PairingFlow pairing;
}

/// BLE setup: put the feeder in setup mode, give it its Wi-Fi, name it and this
/// phone, then pick it and key in the code. The name and Wi-Fi come first
/// because the setup session carries them with the code.
class SetupFlow extends ChangeNotifier {
  /// Setup of a feeder saved to `_book`.
  SetupFlow(this._book, this._ports, this._phone);

  final FeederBook _book;
  final PairingPorts _ports;
  final PhonePorts _phone;

  WifiCredentials? _wifi;
  String _feederName = PairingFlow.defaultFeederName;
  PairingFlow? _pairing;
  bool _disposed = false;

  /// How setup stands.
  SetupState get state => _state;
  SetupState _state = const SetupMode();

  /// How feeders list this phone so far.
  String get label => _book.label;

  /// The permissions step, only when something is still to grant.
  Future<void> load() async {
    if (!await _phone.setupGranted()) _set(const SetupPermissions());
  }

  /// Ask Android; whether Bluetooth came.
  Future<bool> grant() async {
    if (!await _phone.askForSetup()) return false;
    _set(const SetupMode());
    return true;
  }

  /// Open this app's page in Android settings.
  Future<void> openSettings() => _phone.openSettings();

  /// The Wi-Fi this phone is on, when Android will say.
  Future<String?> currentNetwork() => _phone.currentNetwork();

  /// The networks this phone hears, for the feeder's Wi-Fi.
  Future<NetworkScan> scanNetworks() => _phone.scanNetworks();

  /// The phone's model, for a name.
  Future<String?> phoneModel() => _phone.model();

  /// Back to how to put the feeder in setup mode.
  void toSetupMode() => _set(const SetupMode());

  /// On to, or back to, the Wi-Fi.
  void toWifi() => _set(SetupWifi(wifi: _wifi));

  /// Take the Wi-Fi, or null to keep the feeder's own.
  void wifiGiven(WifiCredentials? wifi) {
    _wifi = wifi;
    _set(SetupNames(feederName: _feederName));
  }

  /// Save the names; why they could not be saved, or null.
  Future<String?> named(String label, String? feeder) {
    _feederName = feeder ?? PairingFlow.defaultFeederName;
    return saveLabel(_book, label);
  }

  /// Pair, with a fresh code; whether Bluetooth came. With `again`, at the
  /// feeder the last try connected to: it stays in setup mode, so nobody has to
  /// pick it twice.
  Future<bool> pair({bool again = false}) async {
    if (!await _phone.askForBluetooth()) return false;
    final PairingFlow? last = _pairing;
    final DiscoveredDevice? device = again ? last?.chosen : null;
    last?.dispose();
    final pairing = PairingFlow(_book, _ports);
    _pairing = pairing;
    _set(SetupPairing(pairing: pairing));
    await pairing.runSetup(wifi: _wifi, name: _feederName, device: device);
    return true;
  }

  void _set(SetupState next) {
    if (_disposed) return;
    _state = next;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _pairing?.dispose();
    super.dispose();
  }
}

/// How LAN recovery stands.
sealed class RecoverState {
  /// A state of recovery.
  const RecoverState();
}

/// How feeders list this phone, when it has no name yet.
final class RecoverName extends RecoverState {
  /// The name step.
  const RecoverName();
}

/// The feeders answering on this network, to pick one.
final class RecoverPick extends RecoverState {
  /// The list so far.
  const RecoverPick({this.found = const []});

  /// Who has answered.
  final List<FoundFeeder> found;
}

/// The pairing itself.
final class RecoverPairing extends RecoverState {
  /// A pairing under way.
  const RecoverPairing({required this.pairing});

  /// Its controller.
  final PairingFlow pairing;
}

/// LAN recovery: add this phone as an owner of a feeder on this network whose
/// window is open.
class RecoverFlow extends ChangeNotifier {
  /// Recovery of a feeder saved to `_book`.
  RecoverFlow(this._book, this._ports, this._phone) {
    if (_book.label.isEmpty) {
      _state = const RecoverName();
    } else {
      _browse();
    }
  }

  final FeederBook _book;
  final PairingPorts _ports;
  final PhonePorts _phone;

  final _found = <String, FoundFeeder>{};
  StreamSubscription<FoundFeeder>? _browsing;
  PairingFlow? _pairing;
  bool _disposed = false;

  /// How recovery stands.
  RecoverState get state => _state;
  RecoverState _state = const RecoverPick();

  /// How feeders list this phone so far.
  String get label => _book.label;

  /// The phone's model, for a name.
  Future<String?> phoneModel() => _phone.model();

  /// Save this phone's name and go on; why it could not be saved, or null.
  Future<String?> named(String label) async {
    final String? failed = await saveLabel(_book, label);
    if (failed == null) _browse();
    return failed;
  }

  /// One browse at a time: a second would outlive the pick, and its next
  /// sighting would pull the screen off the code mid-pairing.
  void _browse() {
    if (_browsing != null) return;
    _set(const RecoverPick());
    _browsing = _ports.browse().listen((feeder) {
      _found[feeder.identity] = feeder;
      _set(RecoverPick(found: List.unmodifiable(_found.values)));
    }, onError: (Object _) {});
  }

  /// Pair with the feeder at `address`, saved under `name`.
  Future<void> pair(String address, String name) async {
    unawaited(_browsing?.cancel());
    _browsing = null;
    _pairing?.dispose();
    final pairing = PairingFlow(_book, _ports);
    _pairing = pairing;
    _set(RecoverPairing(pairing: pairing));
    await pairing.runRecovery(address: address, name: name);
  }

  void _set(RecoverState next) {
    if (_disposed) return;
    _state = next;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(_browsing?.cancel());
    _pairing?.dispose();
    super.dispose();
  }
}
