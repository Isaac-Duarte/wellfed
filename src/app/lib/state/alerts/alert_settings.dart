import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/fleet/app_state.dart';

/// This phone's alert settings, as the settings page shows them.
sealed class AlertSettingsState {
  /// For the modes.
  const AlertSettingsState();
}

/// Not read yet.
final class AlertSettingsLoading extends AlertSettingsState {
  /// Loading.
  const AlertSettingsLoading();
}

/// The settings, read.
final class AlertSettingsShown extends AlertSettingsState {
  /// The choices as `settings` holds them, with Android's word on
  /// notifications.
  AlertSettingsShown({
    required this.notificationsAllowed,
    required this.instant,
    required List<KindLevel> kinds,
    required Set<NoticeKind> mutedKinds,
    required Set<AlertLevel> mutedLevels,
    this.problem,
  }) : kinds = List.unmodifiable(kinds),
       mutedKinds = Set.unmodifiable(mutedKinds),
       mutedLevels = Set.unmodifiable(mutedLevels);

  /// Android lets this app post notifications.
  final bool notificationsAllowed;

  /// "Instant while home" is on.
  final bool instant;

  /// Every kind of alert, with its level, in the order the page lists them.
  final List<KindLevel> kinds;

  /// What this phone does not post.
  final Set<NoticeKind> mutedKinds;

  /// Levels this phone does not post.
  final Set<AlertLevel> mutedLevels;

  /// Why the last change did not take, for the owner.
  final String? problem;

  /// Whether `kind` is muted, by itself or by its level.
  bool muted(KindLevel kind) => mutedKinds.contains(kind.kind) || mutedLevels.contains(kind.level);
}

/// The page's settings could not be read.
final class AlertSettingsFailed extends AlertSettingsState {
  /// Failed, saying `message`.
  const AlertSettingsFailed(this.message);

  /// What failed.
  final String message;
}

/// This phone's choices for alerts: notifications, "instant while home" and
/// the mutes. None of it reaches a feeder.
class AlertSettingsController extends ChangeNotifier {
  /// The settings, through `_ports`, for the feeders in `_book`.
  AlertSettingsController(this._ports, this._book);

  final AlertPorts _ports;
  final FeederBook _book;
  bool _disposed = false;

  /// What the page shows.
  AlertSettingsState get state => _state;
  AlertSettingsState _state = const AlertSettingsLoading();

  /// Read the settings, and Android's word on notifications.
  Future<void> load({String? problem}) async {
    AlertSettingsState next;
    try {
      final AlertSettingsView settings = await _ports.settings();
      next = AlertSettingsShown(
        notificationsAllowed: await _ports.notificationsAllowed(),
        instant: settings.instant,
        kinds: settings.kinds,
        mutedKinds: settings.mutedKinds.toSet(),
        mutedLevels: settings.mutedLevels.toSet(),
        problem: problem,
      );
    } on BridgeError catch (error) {
      next = AlertSettingsFailed(error.message);
    }
    if (_disposed) return;
    _state = next;
    notifyListeners();
  }

  /// Ask Android to let alerts through.
  Future<void> allowNotifications() async {
    await _ports.askForNotifications();
    await load();
  }

  /// Turn "instant while home" on or off.
  Future<void> setInstant({required bool on}) async {
    String? problem;
    try {
      final bool went = await _ports.setInstant(on: on);
      if (on && !went) problem = "Android didn't let it start. Open Wellfed and try again.";
    } on PlatformException catch (error) {
      problem = "Couldn't change it: ${error.message ?? error.code}.";
    } on BridgeError catch (error) {
      problem = "Couldn't save it: ${error.message}.";
    }
    await load(problem: problem);
  }

  /// Mute a kind of alert, or unmute it.
  Future<void> muteKind(NoticeKind kind, {required bool muted}) => _mute(MuteTarget.kind(kind: kind), muted: muted);

  /// Mute a level of alert, or unmute it.
  Future<void> muteLevel(AlertLevel level, {required bool muted}) =>
      _mute(MuteTarget.level(level: level), muted: muted);

  Future<void> _mute(MuteTarget target, {required bool muted}) async {
    String? problem;
    try {
      await _ports.mute(target, muted: muted, feeders: _book.feeders);
    } on BridgeError catch (error) {
      problem = "Couldn't save it: ${error.message}.";
    }
    await load(problem: problem);
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
