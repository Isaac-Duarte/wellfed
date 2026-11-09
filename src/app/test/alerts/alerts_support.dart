import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/store.dart';

const kitchen = KnownFeeder(identity: 'ab', name: 'Kitchen');

/// A failed meal: 1 of 2 portions, jammed.
NoticeView failedMeal({int id = 0x40000001, bool quiet = false}) => NoticeView(
  id: id,
  kind: NoticeKind.mealFailed,
  level: AlertLevel.urgent,
  atMs: DateTime(2026, 11, 9, 8).millisecondsSinceEpoch,
  seenMs: DateTime(2026, 11, 9, 8, 1).millisecondsSinceEpoch,
  detail: const NoticeDetail.feed(
    feed: 3,
    plan: 0,
    requested: 2,
    completed: 1,
    outcome: FeedOutcome.failed,
    reason: FeedReason.jammed,
  ),
  quiet: quiet,
);

NoticeView foodLow({int id = 0x40000002}) => NoticeView(
  id: id,
  kind: NoticeKind.foodLow,
  level: AlertLevel.soon,
  seenMs: DateTime(2026, 11, 9, 7).millisecondsSinceEpoch,
  detail: const NoticeDetail.plain(),
  quiet: true,
);

/// Every kind with its level, as Rust lists them.
const kinds = [
  KindLevel(kind: NoticeKind.mealFailed, level: AlertLevel.urgent),
  KindLevel(kind: NoticeKind.foodLow, level: AlertLevel.soon),
  KindLevel(kind: NoticeKind.clientJoined, level: AlertLevel.security),
  KindLevel(kind: NoticeKind.unreachable, level: AlertLevel.urgent),
];

/// Alert ports in memory: a list per feeder, and the settings.
class FakeAlertPorts implements AlertPorts {
  List<NoticeView> notices = [];
  int unread = 0;
  int? reachedMs;
  bool instant = false;
  final mutedKinds = <NoticeKind>{};
  final mutedLevels = <AlertLevel>{};
  bool allowed = true;
  bool instantRefused = false;

  /// Whether instant mode runs, and whether Android starts it again.
  bool running = false;
  bool resumes = true;
  final asked = <List<KnownFeeder>>[];
  final postedOpen = <List<KnownFeeder>>[];

  /// What the next read or change throws, as a damaged store would.
  BridgeError? fails;
  final checked = <List<KnownFeeder>>[];
  final dismissed = <int>[];

  void _fail() {
    if (fails case final BridgeError error) throw error;
  }

  @override
  Future<FeederAlerts> alerts(String identity) async {
    _fail();
    return FeederAlerts(notices: notices, unread: unread, reachedMs: reachedMs);
  }

  @override
  Future<void> check(List<KnownFeeder> feeders) async {
    _fail();
    checked.add(feeders);
    reachedMs = DateTime(2026, 11, 9, 9).millisecondsSinceEpoch;
  }

  @override
  Future<void> dismiss(KnownFeeder feeder, NoticeView notice) async {
    _fail();
    dismissed.add(notice.id);
    notices = [
      for (final each in notices)
        if (each.id != notice.id) each,
    ];
  }

  @override
  Future<AlertSettingsView> settings() async {
    _fail();
    return AlertSettingsView(
      instant: instant,
      kinds: kinds,
      mutedKinds: mutedKinds.toList(),
      mutedLevels: mutedLevels.toList(),
    );
  }

  @override
  Future<void> mute(MuteTarget target, {required bool muted, required List<KnownFeeder> feeders}) async {
    _fail();
    switch (target) {
      case MuteTarget_Kind(:final NoticeKind kind):
        muted ? mutedKinds.add(kind) : mutedKinds.remove(kind);
      case MuteTarget_Level(:final AlertLevel level):
        muted ? mutedLevels.add(level) : mutedLevels.remove(level);
    }
  }

  @override
  Future<bool> setInstant({required bool on}) async {
    final bool went = !(on && instantRefused);
    instant = on && went;
    running = instant;
    return went;
  }

  @override
  Future<bool> instantRunning() async => running;

  @override
  Future<bool> resumeInstant() async => running = resumes;

  @override
  Future<bool> notificationsAllowed() async => allowed;

  @override
  Future<void> postOpen(List<KnownFeeder> feeders) async => postedOpen.add(feeders);

  @override
  bool asking = false;

  @override
  Future<bool> askForNotifications(List<KnownFeeder> feeders) async {
    asked.add(feeders);
    return allowed = true;
  }
}
