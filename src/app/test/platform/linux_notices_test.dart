// The Linux plugin loads libc.so.6 when it is built, so this runs on Linux only.
@TestOn('linux')
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/platform/notices.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/frb_generated.dart';

import '../alerts/alerts_support.dart';
import '../support/fakes.dart';

/// Linux's side of the plugin as 8.0.2 has it: no list of what shows and no
/// launch details, both of which throw, and a server that can go away.
class _Linux extends LinuxFlutterLocalNotificationsPlugin {
  final shown = <int>[];
  final cancelled = <int>[];
  final kept = <int, int>{};
  bool serverGone = false;

  @override
  Future<bool?> initialize({
    required LinuxInitializationSettings settings,
    DidReceiveNotificationResponseCallback? onDidReceiveNotificationResponse,
  }) async => true;

  @override
  Future<void> show({
    required int id,
    String? title,
    String? body,
    LinuxNotificationDetails? notificationDetails,
    String? payload,
  }) async {
    shown.add(id);
    kept[id] = id + 1000;
  }

  @override
  Future<void> cancel({required int id}) async {
    if (serverGone) throw Exception('org.freedesktop.DBus.Error.ServiceUnknown');
    cancelled.add(id);
    kept.remove(id);
  }

  @override
  Future<LinuxServerCapabilities> getCapabilities() async {
    if (serverGone) throw Exception('org.freedesktop.DBus.Error.ServiceUnknown');
    return const LinuxServerCapabilities(
      otherCapabilities: {},
      body: true,
      bodyHyperlinks: false,
      bodyImages: false,
      bodyMarkup: false,
      iconMulti: false,
      iconStatic: true,
      persistence: true,
      sound: false,
      actions: true,
      actionIcons: false,
    );
  }

  @override
  Future<Map<int, int>> getSystemIdMap() async => Map.of(kept);

  @override
  Future<List<ActiveNotification>> getActiveNotifications() =>
      throw UnimplementedError('getActiveNotifications() has not been implemented');

  @override
  Future<NotificationAppLaunchDetails?> getNotificationAppLaunchDetails() =>
      throw UnimplementedError('getNotificationAppLaunchDetails() has not been implemented');
}

/// The bridge's alert calls in memory.
class _Bridge implements RustLibApi {
  final sweptFrom = <List<int>>[];
  final confirmed = <(List<int>, List<int>)>[];
  List<NoticeChange> changes = [];

  /// Catch-ups running now, and the most at once.
  int running = 0;
  int most = 0;
  Completer<void>? hold;

  @override
  Future<Int32List> crateApiAlertsAlertsToSweep({required String dir, required List<int> showing}) async {
    sweptFrom.add(showing);
    return Int32List.fromList([...showing.where((id) => id == 0x40000009)]);
  }

  @override
  Future<void> crateApiAlertsAlertsSwept({required String dir, required List<int> takenOff}) async {}

  @override
  Future<CatchUp> crateApiAlertsCatchUpWith({
    required String dir,
    required ClientKey key,
    required String identity,
    String? manual,
    String? known,
    required Attempt attempt,
    required PhoneNetwork network,
  }) async {
    running++;
    most = running > most ? running : most;
    await hold?.future;
    running--;
    return CatchUp(end: CatchUpEnd.read, changes: changes);
  }

  @override
  Future<Int32List> crateApiAlertsConfirmNotices({
    required String dir,
    required String identity,
    required List<PostedNotice> posted,
    required List<int> removed,
  }) async {
    confirmed.add(([for (final PostedNotice each in posted) each.id], removed));
    return Int32List(0);
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Key with FakeOpaque implements ClientKey {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final linux = _Linux();
  final bridge = _Bridge();

  Future<void> round() => catchUpRound(
    key: _Key(),
    dir: '/data',
    feeders: [kitchen],
    network: () async => const PhoneNetwork.lan(network: '192.168.1.0/24'),
    failures: RoundFailures.thrown,
  );

  setUpAll(() async {
    RustLib.initMock(api: bridge);
    FlutterLocalNotificationsPlatform.instance = linux;
    Notices.host = NoticeHost.linux;
    debugDefaultTargetPlatformOverride = TargetPlatform.linux;
    await Notices.init();
  });

  tearDownAll(() {
    debugDefaultTargetPlatformOverride = null;
    Notices.host = NoticeHost.current;
  });

  setUp(() {
    linux
      ..serverGone = false
      ..shown.clear()
      ..cancelled.clear()
      ..kept.clear();
    bridge
      ..sweptFrom.clear()
      ..confirmed.clear()
      ..changes = []
      ..hold = null
      ..most = 0;
  });

  test("a sweep reads what Linux's side keeps, and takes off what Rust chose", () async {
    linux.kept.addAll({0x40000001: 7, 0x40000009: 8});

    await Notices.sweep('/data');

    expect(bridge.sweptFrom.single, unorderedEquals([0x40000001, 0x40000009]));
    expect(linux.cancelled, [0x40000009]);
  });

  test('a round posts and takes off through the Linux side, then confirms both', () async {
    linux.kept[0x40000002] = 9;
    bridge.changes = [NoticeChange.post(notice: failedMeal()), const NoticeChange.remove(id: 0x40000002)];

    await round();

    expect(linux.shown, [0x40000001]);
    expect(linux.cancelled, [0x40000002]);
    final (List<int> posted, List<int> removed) = bridge.confirmed.single;
    expect(posted, [0x40000001]);
    expect(removed, [0x40000002]);
  });

  test('a server gone mid-round leaves posts unconfirmed and still confirms what came off', () async {
    linux.serverGone = true;
    bridge.changes = [NoticeChange.post(notice: failedMeal()), const NoticeChange.remove(id: 0x40000002)];

    await round();

    expect(linux.shown, isEmpty, reason: 'no server, so nothing to post to');
    final (List<int> posted, List<int> removed) = bridge.confirmed.single;
    expect(posted, isEmpty);
    expect(removed, [0x40000002], reason: 'what it showed went with it');
  });

  test('a launch is never asked of Linux, which cannot say', () async {
    expect(await Notices.launchedFor(), isNull);
  });

  test('a check during a round waits for it, so nothing posts twice', () async {
    bridge.hold = Completer<void>();
    final Future<void> first = round();
    final Future<void> second = round();
    await pumpEventQueue();
    expect(bridge.running, 1);

    bridge.hold?.complete();
    await Future.wait([first, second]);
    expect(bridge.most, 1);
  });

  test('starting alerts never stops the app opening, whatever fails', () async {
    // The app's folder can't be found here, as path_provider has no host side.
    await expectLater(startComputerAlerts(), completes);
  });
}
