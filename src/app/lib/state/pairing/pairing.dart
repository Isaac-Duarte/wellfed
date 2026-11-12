import 'dart:async';

import 'package:clock/clock.dart';
import 'package:flutter/foundation.dart';

import 'package:flutter_reactive_ble/flutter_reactive_ble.dart' show DiscoveredDevice;

import 'package:wellfed/platform/pairing_ports.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/setup.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/pairing/pairing_state.dart';

export 'package:wellfed/src/rust/api/store.dart' show KnownFeeder;
export 'package:wellfed/state/pairing/pairing_state.dart';

/// One pairing, BLE setup or LAN recovery, from the scan to the feeder
/// answering on the LAN. The protocol runs in Rust; this moves its bytes
/// and tracks what the screen shows.
class PairingFlow extends ChangeNotifier {
  /// A pairing saved to `_app`.
  PairingFlow(this._app, this._ports);

  /// How long the feeder gets to answer each step of the handshake.
  static const handshakeWait = Duration(seconds: 10);

  /// Beyond the feeder's own window, for its reply to arrive.
  static const confirmSlack = Duration(seconds: 10);

  /// A feeder restarts and rejoins its Wi-Fi in about 30 s.
  static const rejoinWait = Duration(seconds: 90);

  final FeederBook _app;
  final PairingPorts _ports;

  /// What this phone calls a feeder until the owner names it.
  static const defaultFeederName = 'Feeder';

  /// How the pairing stands.
  PairingState get state => _state;
  PairingState _state = const PairingFinding(feederName: defaultFeederName);

  /// Show `state` on a screen under test, with no exchange behind it.
  @visibleForTesting
  void show(PairingState state) {
    _state = state;
    notifyListeners();
  }

  /// The feeder the owner connected to, so a new code goes to it without
  /// asking again.
  DiscoveredDevice? get chosen => _target;
  DiscoveredDevice? _target;

  /// The code as panel positions, 1 (Lock) to 4 (Reset) from the left.
  List<int> _code = const [];
  String _feederName = defaultFeederName;

  /// The network the feeder was given, for "Keep this phone on …".
  String? _network;

  /// Feeders in setup mode heard so far, by radio id; each sighting
  /// replaces the last, so the signal is current.
  final _found = <String, DiscoveredDevice>{};

  Completer<DiscoveredDevice>? _chosen;
  BleLink? _link;
  StreamSubscription<List<int>>? _notifications;
  Timer? _timer;
  Completer<SetupStep>? _ended;
  StreamSubscription<SetupStep>? _recovering;
  Recovery? _recovery;
  Completer<void>? _recoveryDone;
  Future<void> _writing = Future.value();
  bool _disposed = false;

  /// BLE setup: `wifi` for the feeder's next join, or `null` to keep its
  /// own; `name` is what this phone calls the feeder. With `device`, the
  /// feeder an earlier try connected to, the search is skipped.
  Future<void> runSetup({required WifiCredentials? wifi, required String name, DiscoveredDevice? device}) async {
    final SetupDriver driver;
    try {
      driver = _ports.newDriver(_app.label, wifi);
    } on BridgeError catch (error) {
      _stop(_failureOf(error), error.message);
      return;
    }
    _code = driver.code();
    _feederName = name;
    _network = wifi?.ssid;

    final DiscoveredDevice target;
    if (device != null) {
      target = device;
    } else {
      _showFinding();
      try {
        target = await _pick();
      } on TimeoutException {
        _stop(PairingFailure.notFound, null);
        return;
      } on _Failed catch (failed) {
        _stop(failed.why, failed.detail);
        return;
      }
    }
    _target = target;
    // A screen that went away must not then hold the feeder's only link.
    if (_disposed) return;

    final SetupStep end;
    try {
      _set(PairingConnecting(feederName: _feederName, network: _network));
      final BleLink link = await _ports.open(target);
      _link = link;
      if (_disposed) return;

      final ended = Completer<SetupStep>();
      // A failure after setup stopped waiting is nobody's error.
      ended.future.ignore();
      _ended = ended;
      _notifications = link.notifications.listen(
        (bytes) => _received(driver, bytes),
        onError: (Object error) => _fail(PairingFailure.link, '$error'),
        onDone: () => _fail(PairingFailure.link, 'the feeder dropped the link'),
      );
      _arm(handshakeWait);
      await link.write(driver.start());
      end = await ended.future;
    } on Object catch (error) {
      await _closeLink();
      // Once the feeder proved the code and has the last step, it goes on
      // without the link: a drop or a wait that ends says nothing of it.
      if (driver.proven() case final Uint8List feeder?) {
        await _finish(SetupStep.unsettled(feeder: feeder), knownAddress: null);
        return;
      }
      switch (error) {
        case TimeoutException():
          _stop(PairingFailure.timeout, null);
        case _Failed(:final why, :final detail):
          _stop(why, detail);
        case BridgeError():
          _stop(_failureOf(error), error.message);
        default:
          _stop(PairingFailure.link, '$error');
      }
      return;
    } finally {
      await _closeLink();
    }
    await _finish(end, knownAddress: null);
  }

  /// LAN recovery with the feeder at `address`, whose window is open.
  Future<void> runRecovery({required String address, required String name}) async {
    _feederName = name;
    final Recovery recovery;
    try {
      recovery = _ports.newRecovery(_app.label);
    } on BridgeError catch (error) {
      _stop(_failureOf(error), error.message);
      return;
    }
    _code = recovery.code();
    _recovery = recovery;
    _set(PairingConnecting(feederName: _feederName, network: _network));

    SetupStep? end;
    final done = Completer<void>();
    _recoveryDone = done;
    // Rust reports a failed recovery as a step. Anything else that arrives
    // as an error (a panic, a lost stream) is still a screen to show, not an
    // exception for nobody to catch.
    // Held so dispose can cancel it: the stream waits out the code window.
    _recovering = recovery
        .run(address: address)
        .listen(
          (step) {
            switch (step) {
              case SetupStep_EnterCode(:final seconds):
                _showCode(seconds);
              case SetupStep_Failed(:final error):
                if (!done.isCompleted) done.completeError(error);
              default:
                end = step;
            }
          },
          onError: (Object error) {
            if (!done.isCompleted) done.completeError(error);
          },
          onDone: () {
            if (!done.isCompleted) done.complete();
          },
        );
    try {
      await done.future;
    } on BridgeError catch (error) {
      _stop(_failureOf(error), error.message);
      return;
    } on Object catch (error) {
      _stop(PairingFailure.other, '$error');
      return;
    } finally {
      _timer?.cancel();
      _recovering = null;
    }
    final ended = end;
    // The feeder took this phone, or may have, whatever the screen does:
    // it is saved even when the screen went meanwhile.
    if (_disposed && ended is! SetupStep_Done && ended is! SetupStep_Unsettled) return;
    if (ended == null) {
      _stop(PairingFailure.other, 'the feeder ended without an answer');
      return;
    }
    await _finish(ended, knownAddress: address);
  }

  /// Scan until the owner picks a feeder; a timeout when none shows up.
  Future<DiscoveredDevice> _pick() async {
    final chosen = Completer<DiscoveredDevice>();
    _chosen = chosen;
    final StreamSubscription<DiscoveredDevice> scanning = _ports.scan().listen(
      (device) {
        _found[device.id] = device;
        _showFinding();
      },
      onError: (Object error) {
        if (!chosen.isCompleted) {
          chosen.completeError(_Failed(PairingFailure.link, '$error'));
        }
      },
    );
    final nothing = Timer(SetupLink.findTimeout, () {
      if (_found.isEmpty && !chosen.isCompleted) {
        chosen.completeError(TimeoutException(null));
      }
    });
    try {
      return await chosen.future;
    } finally {
      nothing.cancel();
      _chosen = null;
      await scanning.cancel();
    }
  }

  /// Connect to `device`, one of [PairingFinding.found].
  void choose(DiscoveredDevice device) {
    final Completer<DiscoveredDevice>? chosen = _chosen;
    if (chosen != null && !chosen.isCompleted) chosen.complete(device);
  }

  /// One notification: hand it to the session and act on what it says.
  Future<void> _received(SetupDriver driver, List<int> bytes) async {
    final Completer<SetupStep>? ended = _ended;
    if (ended == null || ended.isCompleted) return;
    try {
      for (final SetupStep step in driver.notified(bytes: bytes)) {
        switch (step) {
          case SetupStep_Write(:final pieces):
            _arm(handshakeWait);
            // Notifications can arrive while an earlier answer is still
            // being written; each answer goes out whole, in order.
            final BleLink? link = _link;
            if (link != null) {
              _writing = _writing.then((_) => link.write(pieces));
              await _writing;
            }
          case SetupStep_EnterCode(:final seconds):
            _showCode(seconds);
          case SetupStep_Done() || SetupStep_Refused() || SetupStep_Unsettled():
            _timer?.cancel();
            ended.complete(step);
          case SetupStep_Failed(:final error):
            _fail(_failureOf(error), error.message);
        }
      }
    } on BridgeError catch (error) {
      _fail(_failureOf(error), error.message);
    } on Object catch (error) {
      _fail(PairingFailure.link, '$error');
    }
  }

  void _showCode(int seconds) {
    _arm(Duration(seconds: seconds) + confirmSlack);
    _set(
      PairingCode(
        code: List.unmodifiable(_code),
        deadline: clock.now().add(Duration(seconds: seconds)),
        feederName: _feederName,
        network: _network,
      ),
    );
  }

  /// After the exchange: remember the feeder, then find it on the LAN.
  Future<void> _finish(SetupStep end, {required String? knownAddress}) async {
    switch (end) {
      case SetupStep_Refused(refusal: final why):
        _stop(PairingFailure.refused, null, refusal: why);
        return;
      case SetupStep_Done(enrolled: final how, :final wifi, feeder: final identity):
        _set(PairingJoining(wifiSaved: wifi, feederName: _feederName, network: _network));
        final KnownFeeder? known = await _save(identity, knownAddress);
        if (known == null) return;
        _set(PairingDone(enrolled: how, feeder: known, feederName: _feederName, network: _network));
      case SetupStep_Unsettled(feeder: final identity):
        _set(PairingSettling(feederName: _feederName, network: _network));
        final String hex = _ports.hexOf(identity);
        final bool wasKnown = _app.feeders.any((feeder) => feeder.identity == hex);
        final KnownFeeder? known = await _save(identity, knownAddress, settling: true);
        if (known == null) return;
        final String? address = known.address;
        final SetupSettled settled = address == null ? SetupSettled.unknown : await _ports.settle(identity, address);
        switch (settled) {
          case SetupSettled.enrolled:
            _set(PairingDone(enrolled: null, feeder: known, feederName: _feederName, network: _network));
          case SetupSettled.notEnrolled:
            // It never took this phone, so it is no feeder of this phone's.
            if (!wasKnown) await _app.forget(hex);
            _stop(PairingFailure.notTaken, null);
          case SetupSettled.unknown:
            _stop(PairingFailure.unsettled, null);
        }
      case SetupStep_Failed(:final error):
        _stop(_failureOf(error), error.message);
      case SetupStep_Write() || SetupStep_EnterCode():
        _stop(PairingFailure.other, 'the session ended mid-exchange');
    }
  }

  /// Save the feeder proved by `identity` before anything else (the proof
  /// is the only way back to it without another setup), then find it on
  /// the LAN, where it is after BLE setup's restart, and save where. `null`
  /// once the screen says it could not be saved.
  Future<KnownFeeder?> _save(List<int> identity, String? knownAddress, {bool settling = false}) async {
    final String hex = _ports.hexOf(identity);
    KnownFeeder known = KnownFeeder(identity: hex, name: _feederName, address: knownAddress);
    try {
      await _app.remember(known, paired: true);
      if (_disposed) return known;
      final String? address = knownAddress ?? await _ports.locate(hex, rejoinWait);
      if (address == null) {
        if (!settling) _stop(PairingFailure.notOnLan, null);
        return settling ? known : null;
      }
      known = KnownFeeder(identity: hex, name: _feederName, address: address);
      if (knownAddress == null) await _app.remember(known, paired: true);
    } on BridgeError catch (error) {
      // The feeder took this phone, or may have; a failed save is said so,
      // not left on "joining".
      _stop(PairingFailure.other, 'paired, but not saved: ${error.message}');
      return null;
    }
    return known;
  }

  /// Fail after `wait` with no message from the feeder.
  void _arm(Duration wait) {
    _timer?.cancel();
    _timer = Timer(wait, () => _fail(PairingFailure.timeout, null));
  }

  void _fail(PairingFailure why, String? detail) {
    final Completer<SetupStep>? ended = _ended;
    if (ended != null && !ended.isCompleted) {
      ended.completeError(_Failed(why, detail));
    }
  }

  Future<void> _closeLink() async {
    _timer?.cancel();
    // Taken first: the screen closing and setup ending may both get here.
    final BleLink? link = _link;
    _link = null;
    unawaited(_notifications?.cancel());
    _notifications = null;
    await link?.close();
  }

  PairingFailure _failureOf(BridgeError error) => switch (error.kind) {
    ErrorKind.codeMismatch => PairingFailure.mismatch,
    ErrorKind.timeout => PairingFailure.timeout,
    ErrorKind.closed => PairingFailure.windowClosed,
    ErrorKind.label || ErrorKind.credentials => PairingFailure.input,
    _ => PairingFailure.other,
  };

  void _stop(PairingFailure why, String? detail, {SetupRefusal? refusal}) =>
      _set(PairingFailed(failure: why, message: detail, refusal: refusal, feederName: _feederName, network: _network));

  void _showFinding() =>
      _set(PairingFinding(found: Map.unmodifiable(_found), feederName: _feederName, network: _network));

  void _set(PairingState next) {
    _state = next;
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    final Completer<DiscoveredDevice>? choosing = _chosen;
    if (choosing != null && !choosing.isCompleted) {
      choosing.completeError(const _Failed(PairingFailure.other, 'the screen closed'));
    }
    _fail(PairingFailure.other, 'the screen closed');
    // Hang up in Rust too, so the feeder's window is free for a retry.
    _recovery?.close();
    unawaited(_recovering?.cancel());
    final Completer<void>? recovering = _recoveryDone;
    if (recovering != null && !recovering.isCompleted) recovering.complete();
    unawaited(_closeLink());
    super.dispose();
  }
}

class _Failed implements Exception {
  const _Failed(this.why, this.detail);

  final PairingFailure why;
  final String? detail;
}
