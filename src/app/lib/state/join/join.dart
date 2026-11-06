import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/join_ports.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/join.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/form_factor.dart';

/// How joining with an invite stands.
sealed class JoinState {
  /// A state of a join.
  const JoinState();
}

/// Waiting for a code.
final class JoinScanning extends JoinState {
  /// The scanner, with what was wrong with the last code.
  const JoinScanning({this.badText});

  /// Why the last code was not used.
  final String? badText;
}

/// The invite, what is known of its feeder, and the owner's word.
final class JoinReview extends JoinState {
  /// The review step.
  const JoinReview({
    this.address,
    this.finding = false,
    this.joining = false,
    this.problem,
    this.refusal,
    this.spent = false,
  });

  /// Where the feeder answers, once found.
  final String? address;

  /// NSD is looking for it.
  final bool finding;

  /// The join went out.
  final bool joining;

  /// Why this join came to nothing.
  final String? problem;

  /// What the feeder refused, when it did.
  final FeederRefusal? refusal;

  /// The invite may be spent or was refused: only a new one can go on.
  final bool spent;
}

/// The feeder took this phone.
final class JoinJoined extends JoinState {
  /// A finished join.
  const JoinJoined({required this.role});

  /// What this phone may do on it.
  final ClientRole role;
}

/// Join a feeder with someone's invite. An invite works once, so a join whose
/// answer never came is not sent again: the state says so and asks for a new one.
class JoinFlow extends ChangeNotifier {
  /// A join saved to `_book`.
  JoinFlow(this._book, this._ports);

  /// How long NSD gets to find an invited feeder before the owner is asked.
  static const _findWait = Duration(seconds: 12);

  final FeederBook _book;
  final JoinPorts _ports;

  String? _invite;
  String _identity = '';
  bool _disposed = false;

  /// How the join stands.
  JoinState get state => _state;
  JoinState _state = const JoinScanning();

  /// How feeders list this phone so far.
  String get label => _book.label;

  /// Take `text`, which a scanner saw or the owner pasted.
  void seen(String text) {
    if (_state is! JoinScanning) return;
    final String? identity = _ports.inviteFeeder(text);
    if (identity == null) {
      _set(const JoinScanning(badText: "That isn't a Wellfed invite."));
      return;
    }
    _invite = text;
    _identity = identity;
    _set(const JoinReview());
    unawaited(find());
  }

  /// Look for the invited feeder on this network.
  Future<void> find() async {
    if (_state case final JoinReview review) {
      _set(
        JoinReview(
          finding: true,
          address: review.address,
          problem: review.problem,
          refusal: review.refusal,
          spent: review.spent,
        ),
      );
    }
    final String? address = await _ports.locate(_identity, _findWait);
    if (_state case final JoinReview review) {
      _set(JoinReview(address: address, problem: review.problem, refusal: review.refusal, spent: review.spent));
    }
  }

  /// Send the invite, once, under these names.
  Future<void> join({required String label, required String feederName}) async {
    final String? invite = _invite;
    final JoinState now = _state;
    if (invite == null || now is! JoinReview) return;
    final String? address = now.address;
    if (address == null) return;

    _set(JoinReview(address: address, joining: true, spent: now.spent));
    try {
      final JoinAnswer answer = await _ports.join(invite, label, address);
      switch (answer) {
        case JoinAnswer_Joined(:final ClientRole role):
          await _book.setLabel(label);
          await _book.remember(KnownFeeder(identity: _identity, name: feederName, address: address));
          _set(JoinJoined(role: role));
        case JoinAnswer_Refused(:final FeederRefusal refusal):
          _set(JoinReview(address: address, refusal: refusal, spent: true));
      }
    } on BridgeError catch (error) {
      _stop(address, switch (error.kind) {
        ErrorKind.connect => "Couldn't reach the feeder. Nothing was sent.",
        ErrorKind.closed =>
          'The feeder turned this invite down: it has expired or been used. '
              'Ask for a new one.',
        ErrorKind.label => 'A phone name is 1 to 16 bytes.',
        ErrorKind.store => 'Joined, but this $deviceName could not save it.',
        ErrorKind.interrupted =>
          'The connection broke, so you may or may not have joined. Wellfed '
              "won't send this invite again. Ask for a new one.",
        ErrorKind.timeout =>
          "No answer came, so you may or may not have joined. Wellfed won't "
              'send this invite again. Ask for a new one.',
        _ => 'That did not work: ${error.message}',
      }, spent: error.kind != ErrorKind.connect && error.kind != ErrorKind.label);
    }
  }

  void _stop(String address, String problem, {required bool spent}) =>
      _set(JoinReview(address: address, problem: problem, spent: spent));

  void _set(JoinState next) {
    if (_disposed) return;
    _state = next;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
