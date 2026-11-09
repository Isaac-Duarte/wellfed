import 'dart:async';
import 'dart:typed_data';

import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/alerts/alert_words.dart';

/// Alerts as Android notifications, one channel per level, so the owner can
/// also silence a level in Android's own settings. A notification has no
/// actions: a tap opens its feeder's alerts and does nothing else.
class Notices {
  static final _plugin = FlutterLocalNotificationsPlugin();
  static final _tapped = StreamController<String>.broadcast();
  static bool _ready = false;

  static const _icon = '@mipmap/ic_launcher';

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
  static Future<void> init() async {
    if (_ready) return;
    await _plugin.initialize(
      settings: const InitializationSettings(android: AndroidInitializationSettings(_icon)),
      onDidReceiveNotificationResponse: (response) {
        if (response.payload case final String identity) _tapped.add(identity);
      },
    );
    final AndroidFlutterLocalNotificationsPlugin? android = _plugin
        .resolvePlatformSpecificImplementation<AndroidFlutterLocalNotificationsPlugin>();
    for (final AlertLevel level in AlertLevel.values) {
      await android?.createNotificationChannel(_channel(level));
    }
    _ready = true;
  }

  /// The feeder a tap asks to open, while the app runs.
  static Stream<String> get tapped => _tapped.stream;

  /// The feeder whose notification opened the app, if one did.
  static Future<String?> launchedFor() async {
    final NotificationAppLaunchDetails? details = await _plugin.getNotificationAppLaunchDetails();
    if (details == null || !details.didNotificationLaunchApp) return null;
    return details.notificationResponse?.payload;
  }

  /// Whether Android lets this app post them.
  static Future<bool> allowed() async =>
      await _plugin
          .resolvePlatformSpecificImplementation<AndroidFlutterLocalNotificationsPlugin>()
          ?.areNotificationsEnabled() ??
      false;

  /// Ask Android to let this app post them: whether it does, or `null` when
  /// Android did not say.
  static Future<bool?> ask() =>
      _plugin
          .resolvePlatformSpecificImplementation<AndroidFlutterLocalNotificationsPlugin>()
          ?.requestNotificationsPermission() ??
      Future.value();

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
    final posted = <int>[];
    final removed = <int>[];
    for (final NoticeChange change in changes) {
      switch (change) {
        case NoticeChange_Remove(:final int id):
          await _plugin.cancel(id: id);
          removed.add(id);
        case NoticeChange_Post(:final NoticeView notice):
          // Muted kinds are never posted; only a phone with notifications
          // off leaves one unconfirmed, for when they are back on.
          if (!await allowed()) continue;
          await _show(identity, name, notice);
          posted.add(notice.id);
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
    final List<int> showing = alertNotificationIds(await _plugin.getActiveNotifications());
    // Empty once the sweep is done, and then nothing changes.
    final List<int> stale = await alertsToSweep(dir: dir, showing: Int32List.fromList(showing));
    await remove(stale);
    await alertsSwept(dir: dir);
  }

  static Future<void> _show(String identity, String name, NoticeView notice) async {
    final AndroidNotificationChannel channel = _channel(notice.level);
    final NoticeWords words = noticeWords(notice, name);
    final List<ActiveNotification> showing = await _plugin.getActiveNotifications();
    await _plugin.show(
      id: notice.id,
      title: words.title,
      body: words.body,
      payload: identity,
      notificationDetails: NotificationDetails(
        android: AndroidNotificationDetails(
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
        ),
      ),
    );
  }

  /// Take notifications off: a feeder this phone forgot, or a sweep.
  static Future<void> remove(Iterable<int> ids) async {
    for (final int id in ids) {
      await _plugin.cancel(id: id);
    }
  }
}

/// The alert notifications among `active`: those on an alert channel.
List<int> alertNotificationIds(List<ActiveNotification> active) => [
  for (final ActiveNotification each in active)
    if (each case ActiveNotification(id: final int id, channelId: final String channel)
        when channel.startsWith('alerts-'))
      id,
];

/// Whether `active` shows notification `id` with these very words already, so
/// posting it again need not alert.
bool showsWords(List<ActiveNotification> active, int id, NoticeWords words) =>
    active.any((each) => each.id == id && each.title == words.title && (each.bigText ?? each.body) == words.body);
