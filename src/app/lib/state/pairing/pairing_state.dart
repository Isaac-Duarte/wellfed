import 'package:flutter_reactive_ble/flutter_reactive_ble.dart' show DiscoveredDevice;

import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/store.dart';

/// Why pairing stopped.
enum PairingFailure {
  /// No feeder in setup mode answered the scan.
  notFound,

  /// The radio link failed or dropped.
  link,

  /// The feeder said no; see [PairingFailed.refusal].
  refused,

  /// The feeder did not prove the code: mistyped, or another device.
  mismatch,

  /// No answer in time.
  timeout,

  /// The recovery window was not open (the feeder closed the connection).
  windowClosed,

  /// Paired, but the feeder never showed up on this network.
  notOnLan,

  /// The phone's name or the Wi-Fi fails the feeder's rules.
  input,

  /// Anything else; [PairingFailed.message] has the detail.
  other,
}

/// How one pairing stands.
sealed class PairingState {
  /// A state of a pairing for [feederName].
  const PairingState({required this.feederName, this.network});

  /// What this phone calls the feeder; only this phone uses it.
  final String feederName;

  /// The network the feeder was given, for "Keep this phone on …".
  final String? network;
}

/// Looking for a feeder in setup mode.
final class PairingFinding extends PairingState {
  /// A search that has heard [found].
  const PairingFinding({required super.feederName, super.network, this.found = const {}});

  /// Feeders in setup mode heard so far, by radio id.
  final Map<String, DiscoveredDevice> found;
}

/// Opening a session.
final class PairingConnecting extends PairingState {
  /// A session on its way.
  const PairingConnecting({required super.feederName, super.network});
}

/// Showing the code; the feeder takes presses until [deadline].
final class PairingCode extends PairingState {
  /// A code on the screen.
  const PairingCode({required this.code, required this.deadline, required super.feederName, super.network});

  /// The code as panel positions, 1 (Lock) to 4 (Reset) from the left.
  final List<int> code;

  /// When the feeder stops taking presses.
  final DateTime deadline;
}

/// The feeder proved the code and is restarting onto its Wi-Fi; the phone looks
/// for it on the LAN.
final class PairingJoining extends PairingState {
  /// A feeder on its way to the LAN.
  const PairingJoining({required this.wifiSaved, required super.feederName, super.network});

  /// What became of the Wi-Fi the feeder was given.
  final WifiSaved wifiSaved;
}

/// Paired, and the feeder answers on the LAN.
final class PairingDone extends PairingState {
  /// A finished pairing.
  const PairingDone({required this.enrolled, required this.feeder, required super.feederName, super.network});

  /// How the feeder took this phone.
  final Enrolled enrolled;

  /// What this phone saved.
  final KnownFeeder feeder;
}

/// It stopped, as [failure] says.
final class PairingFailed extends PairingState {
  /// A pairing that stopped.
  const PairingFailed({
    required this.failure,
    required super.feederName,
    super.network,
    this.message,
    this.refusal,
  });

  /// Why it stopped.
  final PairingFailure failure;

  /// The detail, where there is one to show.
  final String? message;

  /// What the feeder refused, for [PairingFailure.refused].
  final SetupRefusal? refusal;
}
