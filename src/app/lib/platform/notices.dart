import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/alerts/alert_words.dart';

/// Which side of the notification plugin this host posts through.
enum NoticeHost {
  /// Notification channels, Android's permission, and a shade that holds
  /// every app's notifications.
  android,

  /// The Darwin side: macOS asks the owner once, and a tap brings the app
  /// forward by itself.
  macos,

  /// The freedesktop notification server over D-Bus, which a desktop may
  /// not run at all.
  linux,

  /// Toasts, under the app user model id the plugin registers for an
  /// unpackaged app.
  windows;

  /// This process's host.
  static NoticeHost get current => Platform.isMacOS
      ? macos
      : Platform.isLinux
      ? linux
      : Platform.isWindows
      ? windows
      : android;

  /// Whether this is a computer, whose alerts post only while the app is open.
  bool get desktop => this != android;
}

/// Alerts as notifications. On Android, one channel per level, so the owner
/// can also silence a level in Android's own settings; on a computer, the
/// system's own notifications, posted while the app is open. A notification
/// has no actions: a tap opens its feeder's alerts and does nothing else.
class Notices {
  static final _plugin = FlutterLocalNotificationsPlugin();
  static final _tapped = StreamController<String>.broadcast();
  static bool _ready = false;

  /// The host this posts through: the process's own, unless a test stands
  /// in another.
  @visibleForTesting
  static NoticeHost host = NoticeHost.current;
  static NoticeHost get _host => host;

  static const _icon = '@mipmap/ic_launcher';

  /// The id Windows files the app's toasts under, which the plugin registers
  /// for an unpackaged app in the user's registry, with [_toastActivator] as
  /// the class a toast's click activates. Fixed for good: a new one strands
  /// the toasts and the notification settings the owner made under the old.
  static const toastAppId = 'io.github.isaacduarte.wellfed';
  static const _toastActivator = 'b8528b2d-8e1f-41d1-b158-6df3f7d90c63';

  /// The Windows runner's own half (`windows/runner/toasts.cpp`): whether
  /// Windows lets the app post, which toasts it shows and taking one off,
  /// all of which the plugin does only for a packaged app.
  static const _toasts = MethodChannel('io.github.isaacduarte.wellfed/toasts');

  static const _settings = InitializationSettings(
    android: AndroidInitializationSettings(_icon),
    // macOS asks only when the app does, at the moment it has a feeder to
    // hear from ([ask]), never as the plugin starts.
    macOS: DarwinInitializationSettings(
      requestAlertPermission: false,
      requestSoundPermission: false,
      requestBadgePermission: false,
    ),
    linux: LinuxInitializationSettings(defaultActionName: 'Open alerts'),
    windows: WindowsInitializationSettings(appName: 'Wellfed', appUserModelId: toastAppId, guid: _toastActivator),
  );

  static AndroidNotificationChannel _channel(AlertLevel level) => switch (level) {
    AlertLevel.urgent => const AndroidNotificationChannel(
      'alerts-urgent',
      'Urgent alerts',
      description: 'A meal went wrong, the feeder cannot feed, or this phone cannot reach it at home.',
      importance: Importance.high,
    ),
    AlertLevel.soon => const AndroidNotificationChannel(
      'alerts-soon',
      'Alerts for today',
      description: 'Food, the desiccant, the batteries, power and updates.',
    ),
    AlertLevel.security => const AndroidNotificationChannel(
      'alerts-security',
      'Security alerts',
      description: 'Someone joined a feeder or was removed from it.',
    ),
  };

  /// Get ready in this engine: each one that posts calls this once. Only the
  /// app's own engine hears taps, through [tapped].
  ///
  /// A computer whose plugin refuses to start (no D-Bus session on Linux, or
  /// Windows's plugin saying it didn't) goes on without notifications:
  /// [allowed] says so, and alerts only list in the app. A failure inside
  /// Windows's own registration is the plugin's to survive, not this.
  static Future<void> init() async {
    if (_ready) return;
    try {
      final bool? started = await _plugin.initialize(
        settings: _settings,
        onDidReceiveNotificationResponse: (response) {
          if (response.payload case final String identity) _tapped.add(identity);
        },
      );
      if (_host.desktop && started == false) {
        debugPrint("notifications are off on this computer: the plugin didn't start");
        return;
      }
    } on Exception catch (error) {
      if (!_host.desktop) rethrow;
      debugPrint('notifications are off on this computer: $error');
      return;
    }
    final AndroidFlutterLocalNotificationsPlugin? android = _plugin
        .resolvePlatformSpecificImplementation<AndroidFlutterLocalNotificationsPlugin>();
    for (final AlertLevel level in AlertLevel.values) {
      await android?.createNotificationChannel(_channel(level));
    }
    _ready = true;
  }

  /// The feeder a tap asks to open, while the app runs.
  static Stream<String> get tapped => _tapped.stream;

  /// The feeder whose notification opened the app, if one did. Linux's side
  /// can't say, and a click there reaches only an app that is running.
  static Future<String?> launchedFor() async {
    if (_host == NoticeHost.linux || (_host.desktop && !_ready)) return null;
    final NotificationAppLaunchDetails? details = await _plugin.getNotificationAppLaunchDetails();
    if (details == null || !details.didNotificationLaunchApp) return null;
    return details.notificationResponse?.payload;
  }

  /// Whether this app may post them: Android's or macOS's word, whether a
  /// Linux desktop runs a notification server, and on Windows the app's
  /// notification setting, read by the runner.
  static Future<bool> allowed() async => switch (_host) {
    NoticeHost.android =>
      await _plugin
              .resolvePlatformSpecificImplementation<AndroidFlutterLocalNotificationsPlugin>()
              ?.areNotificationsEnabled() ??
          false,
    NoticeHost.macos =>
      (await _plugin.resolvePlatformSpecificImplementation<MacOSFlutterLocalNotificationsPlugin>()?.checkPermissions())
              ?.isEnabled ??
          false,
    NoticeHost.linux => _ready && await _serverAnswers(),
    NoticeHost.windows => _ready && await _toastsEnabled(),
  };

  /// Whether Windows lets the app's toasts through, as the switches its
  /// Settings app writes say ([toastsAllowed]). A runner that can't read
  /// them, or has no toast half, is believed: Windows posts unless told not
  /// to.
  static Future<bool> _toastsEnabled() async {
    try {
      final Map<String, Object?>? read = await _toasts.invokeMapMethod<String, Object?>('switches', {
        'app': toastAppId,
      });
      return toastsAllowed(app: read?['app'] as int?, all: read?['all'] as int?);
    } on MissingPluginException {
      return true;
    } on PlatformException {
      return true;
    }
  }

  /// How long a notification server may take to answer before it counts as
  /// none: a hung one must not hold every delivery behind it.
  static const _serverWait = Duration(seconds: 5);

  /// Whether a freedesktop notification server answers on the session bus.
  /// A desktop with none (a bare window manager) refuses the call.
  static Future<bool> _serverAnswers() async {
    try {
      await _plugin
          .resolvePlatformSpecificImplementation<LinuxFlutterLocalNotificationsPlugin>()
          ?.getCapabilities()
          .timeout(_serverWait);
      return true;
    } on TimeoutException {
      return false;
    } on Exception {
      return false;
    }
  }

  /// Whether this computer can't post at all, so there is nothing to ask:
  /// Linux with no notification server, or a plugin that didn't start. A
  /// Windows whose settings keep the toasts off can still be asked, through
  /// those settings.
  static Future<bool> unavailable() async => switch (_host) {
    NoticeHost.android || NoticeHost.macos => false,
    NoticeHost.linux => !await allowed(),
    NoticeHost.windows => !_ready,
  };

  /// Ask Android or macOS to let this app post them: whether it does, or
  /// `null` when the system did not say. Linux and Windows ask nothing.
  static Future<bool?> ask() async => switch (_host) {
    NoticeHost.android =>
      await _plugin
          .resolvePlatformSpecificImplementation<AndroidFlutterLocalNotificationsPlugin>()
          ?.requestNotificationsPermission(),
    NoticeHost.macos =>
      await _plugin.resolvePlatformSpecificImplementation<MacOSFlutterLocalNotificationsPlugin>()?.requestPermissions(
        alert: true,
        sound: true,
      ),
    NoticeHost.linux || NoticeHost.windows => allowed(),
  };

  /// Make `changes` for the feeder `identity` called `name`, then tell Rust
  /// which were made, so none is handed over again. A change Android refuses
  /// stays unconfirmed, and the next catch-up hands it over again.
  static Future<void> deliver({
    required String dir,
    required String identity,
    required String name,
    required List<NoticeChange> changes,
  }) async {
    if (changes.isEmpty) return;
    final posted = <PostedNotice>[];
    final removed = <int>[];
    for (final NoticeChange change in changes) {
      switch (change) {
        case NoticeChange_Remove(:final int id):
          await _cancel(id);
          removed.add(id);
        case NoticeChange_Post(:final NoticeView notice):
          // Muted kinds are never posted; only a phone with notifications
          // off leaves one unconfirmed, for when they are back on.
          if (!await allowed()) continue;
          await _show(identity, name, notice);
          // Which posting went up, so a newer one a catch-up made meanwhile
          // under the same id is not taken for it.
          posted.add(PostedNotice(id: notice.id, seenMs: notice.seenMs, kind: notice.kind));
      }
    }
    // A feeder forgotten while these were on the way: what went up comes off.
    final List<int> again = await confirmNotices(dir: dir, identity: identity, posted: posted, removed: removed);
    await remove(again);
  }

  /// Once `alerts.json` was set aside, take off the alert notifications no
  /// watch in the new file knows: nothing else ever would. Rust chooses them
  /// under the file's lock, so one another engine posted meanwhile stays.
  static Future<void> sweep(String dir) async {
    final List<int> showing = await _showing();
    // Empty once the sweep is done, and then nothing changes.
    final List<int> stale = await alertsToSweep(dir: dir, showing: Int32List.fromList(showing));
    await remove(stale);
    await alertsSwept(dir: dir, takenOff: Int32List.fromList(stale));
  }

  /// The alert notifications this app shows now, as each system can say.
  /// Linux's side lists none but those it keeps a server id for; an
  /// unpackaged Windows app reads its toast history through the runner.
  static Future<List<int>> _showing() async {
    if (_host.desktop && !_ready) return const [];
    switch (_host) {
      case NoticeHost.android || NoticeHost.macos:
        return alertNotificationIds(await _plugin.getActiveNotifications(), host: _host);
      case NoticeHost.linux:
        final Map<int, int>? kept = await _plugin
            .resolvePlatformSpecificImplementation<LinuxFlutterLocalNotificationsPlugin>()
            ?.getSystemIdMap();
        return [...?kept?.keys];
      case NoticeHost.windows:
        try {
          return [
            ...?await _toasts.invokeListMethod<int>('shown', {'app': toastAppId}),
          ];
        } on MissingPluginException {
          return const [];
        }
    }
  }

  static Future<void> _show(String identity, String name, NoticeView notice) async {
    final NoticeWords words = noticeWords(notice, name);
    await _plugin.show(
      id: notice.id,
      title: words.title,
      body: words.body,
      payload: identity,
      notificationDetails: switch (_host) {
        NoticeHost.android => NotificationDetails(
          android: _android(notice, words, await _plugin.getActiveNotifications()),
        ),
        NoticeHost.macos || NoticeHost.linux || NoticeHost.windows => desktopDetails(notice, identity),
      },
    );
  }

  static AndroidNotificationDetails _android(NoticeView notice, NoticeWords words, List<ActiveNotification> showing) {
    final AndroidNotificationChannel channel = _channel(notice.level);
    return AndroidNotificationDetails(
      channel.id,
      channel.name,
      channelDescription: channel.description,
      importance: channel.importance,
      priority: notice.level == AlertLevel.urgent ? Priority.high : Priority.defaultPriority,
      // Delivery is at least once: a repeat of the same words under this
      // id must not buzz, but new words (an uncertain meal found failed)
      // must.
      onlyAlertOnce: showsWords(showing, notice.id, words),
      styleInformation: BigTextStyleInformation(words.body),
      when: notice.atMs,
      showWhen: notice.atMs != null,
    );
  }

  /// Take notification `id` off. On a computer a system that fails to (a
  /// Linux notification server gone meanwhile) counts it as off: what it
  /// showed went with it, and a delivery must still reach its confirmation.
  static Future<void> _cancel(int id) async {
    if (!_host.desktop) return _plugin.cancel(id: id);
    if (!_ready) return;
    try {
      await _plugin.cancel(id: id);
      if (_host == NoticeHost.windows) await _toasts.invokeMethod<bool>('takeOff', {'id': id, 'app': toastAppId});
    } on MissingPluginException {
      // A Windows runner with no toast half: the toast stays until the owner
      // clears it.
    } on Exception catch (error) {
      debugPrint('notification $id was not taken off: $error');
    }
  }

  /// Take notifications off: a feeder this phone forgot, or a sweep.
  static Future<void> remove(Iterable<int> ids) async {
    for (final int id in ids) {
      await _cancel(id);
    }
  }
}

/// Whether Windows lets the app's toasts through, from the two switches its
/// Settings app writes: the app's own (`app`) and every app's (`all`). 0 is
/// off; a switch never written (`null`) is on.
bool toastsAllowed({required int? app, required int? all}) => app != 0 && all != 0;

/// How a computer shows `notice` of the feeder `identity`. A computer has no
/// channels: an urgent alert asks for more of the owner's attention instead,
/// and each feeder's alerts group together where the system groups them.
NotificationDetails desktopDetails(NoticeView notice, String identity) {
  final bool urgent = notice.level == AlertLevel.urgent;
  return NotificationDetails(
    macOS: DarwinNotificationDetails(threadIdentifier: identity),
    linux: LinuxNotificationDetails(
      // A critical notification stays until the owner dismisses it.
      urgency: urgent ? LinuxNotificationUrgency.critical : LinuxNotificationUrgency.normal,
      category: urgent ? LinuxNotificationCategory.deviceError : LinuxNotificationCategory.device,
    ),
    windows: const WindowsNotificationDetails(),
  );
}

/// The alert notifications among `active`: on Android those on an alert
/// channel, beside the keep-alive service's; a computer posts nothing else.
List<int> alertNotificationIds(List<ActiveNotification> active, {required NoticeHost host}) => [
  for (final ActiveNotification each in active)
    if (each case ActiveNotification(id: final int id, :final String? channelId)
        when host.desktop || (channelId?.startsWith('alerts-') ?? false))
      id,
];

/// Whether `active` shows notification `id` with these very words already, so
/// posting it again need not alert.
bool showsWords(List<ActiveNotification> active, int id, NoticeWords words) =>
    active.any((each) => each.id == id && each.title == words.title && (each.bigText ?? each.body) == words.body);
