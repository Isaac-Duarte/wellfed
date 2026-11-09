import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/store.dart';

/// One feeder's alerts on this phone, as its alert screen shows them.
sealed class AlertsState {
  /// For the modes.
  const AlertsState();
}

/// Not read yet.
final class AlertsLoading extends AlertsState {
  /// Loading.
  const AlertsLoading();
}

/// What this phone lists, with `checking` set while a catch-up runs.
final class AlertsShown extends AlertsState {
  /// `notices`, `unread` and `reachedMs` as Rust listed them.
  AlertsShown({
    required List<NoticeView> notices,
    required this.unread,
    required this.reachedMs,
    this.checking = false,
    this.problem,
  }) : notices = List.unmodifiable(notices);

  /// Open and not dismissed here, oldest first.
  final List<NoticeView> notices;

  /// Alerts the feeder holds that this app is too old to read.
  final int unread;

  /// When a catch-up last reached the feeder, by this phone's clock.
  final int? reachedMs;

  /// A catch-up runs now.
  final bool checking;

  /// Why the last action failed, for the owner.
  final String? problem;

  /// This, with `checking` and `problem` replaced.
  AlertsShown copyWith({required bool checking, String? problem}) =>
      AlertsShown(notices: notices, unread: unread, reachedMs: reachedMs, checking: checking, problem: problem);
}

/// The phone's alert file could not be read.
final class AlertsFailed extends AlertsState {
  /// Failed, saying `message`.
  const AlertsFailed(this.message);

  /// What failed.
  final String message;
}

/// One feeder's alerts: what this phone lists, a catch-up on demand, and a
/// dismissal that hides one here. Nothing here reaches the feeder but a
/// catch-up's reads; a dismissal is this phone's alone.
class FeederAlertsController extends ChangeNotifier {
  /// The alerts of `_feeder`, through `_ports`.
  FeederAlertsController(this._ports, this._feeder);

  final AlertPorts _ports;
  final KnownFeeder _feeder;
  bool _disposed = false;

  /// What the screen shows.
  AlertsState get state => _state;
  AlertsState _state = const AlertsLoading();

  /// Read what this phone lists.
  Future<void> load() async {
    AlertsState next;
    try {
      final FeederAlerts found = await _ports.alerts(_feeder.identity);
      next = AlertsShown(notices: found.notices, unread: found.unread, reachedMs: found.reachedMs);
    } on BridgeError catch (error) {
      next = AlertsFailed(error.message);
    }
    _set(next);
  }

  /// Catch up with the feeder now, then list again.
  Future<void> check() async {
    final AlertsState shown = _state;
    if (shown is! AlertsShown || shown.checking) return;
    _set(shown.copyWith(checking: true));
    try {
      await _ports.check([_feeder]);
    } on BridgeError catch (error) {
      _set(shown.copyWith(checking: false, problem: "Couldn't check: ${error.message}."));
      return;
    } on PlatformException catch (error) {
      _set(shown.copyWith(checking: false, problem: "Couldn't check: ${error.message ?? error.code}."));
      return;
    }
    await load();
  }

  /// Hide `notice` on this phone until the feeder clears it. The feeder is
  /// never told.
  Future<void> dismiss(NoticeView notice) async {
    final AlertsState shown = _state;
    try {
      await _ports.dismiss(_feeder, notice);
    } on BridgeError catch (error) {
      if (shown is AlertsShown) _set(shown.copyWith(checking: false, problem: "Couldn't dismiss: ${error.message}."));
      return;
    }
    await load();
  }

  void _set(AlertsState next) {
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
