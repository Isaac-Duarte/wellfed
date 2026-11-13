import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';
import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/platform/notices.dart';
import 'package:wellfed/platform/run_holds.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/store.dart';

/// What the alert screens reach outside Dart.
abstract interface class AlertPorts {
  /// The feeder `identity`'s alerts as this phone lists them.
  Future<FeederAlerts> alerts(String identity);

  /// Catch up with `feeders` now.
  Future<void> check(List<KnownFeeder> feeders);

  /// Hide `notice` of `feeder` on this phone, and take its notification off.
  Future<void> dismiss(KnownFeeder feeder, NoticeView notice);

  /// This phone's choices for alerts.
  Future<AlertSettingsView> settings();

  /// Mute `target` or unmute it; notifications of what is muted come off.
  Future<void> mute(MuteTarget target, {required bool muted, required List<KnownFeeder> feeders});

  /// Turn "instant while home" on or off. Whether Android went along.
  Future<bool> setInstant({required bool on});

  /// Whether instant mode runs now. On in the settings, it still stops when
  /// the phone restarts or Android stops it, until the app opens again.
  Future<bool> instantRunning();

  /// Start instant mode again, as it is on in the settings, keeping that
  /// setting whatever Android says. Whether it runs.
  Future<bool> resumeInstant();

  /// Whether Android lets this app post notifications.
  Future<bool> notificationsAllowed();

  /// What the button does while notifications are off.
  Future<NotificationAsk> notificationAsk();

  /// Do what [notificationAsk] says: ask Android, or open its notification
  /// settings. Whether it lets the app post now. Once it does, alerts still
  /// open for `feeders` are posted; a refusal is noted, and the owner stays
  /// where they were.
  Future<bool> askForNotifications(List<KnownFeeder> feeders);

  /// Post what is still open for `feeders`: for notifications allowed in
  /// Android's settings, with the app away.
  Future<void> postOpen(List<KnownFeeder> feeders);

  /// An ask is out: Android's dialog pauses the app and resumes it, and the
  /// ask posts what is open itself once allowed.
  bool get asking;
}

/// The Wi-Fi network as Kotlin described it, or none.
PhoneNetwork phoneNetwork(Map<Object?, Object?>? found) => switch (found) {
  {'gateway': final String gateway, 'subnet': final String subnet} => PhoneNetwork.wifi(
    gateway: gateway,
    subnet: subnet,
  ),
  _ => const PhoneNetwork.elsewhere(),
};

/// How long NSD looks for a feeder that did not answer where it last did:
/// long enough to hear it, short enough for a background job.
const _locateWait = Duration(seconds: 6);

/// What a round does with a feeder's failed catch-up, once every other
/// feeder has had its turn.
enum RoundFailures {
  /// Logged only: a background round has nobody to tell.
  logged,

  /// The first is thrown at the end, for the owner who asked for the check.
  thrown,
}

/// One round of catch-ups with `feeders`, as `key`, with the app's files in
/// `dir`, on the network `network` says the phone is on. Each feeder's
/// changes are posted and confirmed before the next, and one that fails leaves the
/// others their turn ([feederRound]).
Future<void> catchUpRound({
  required ClientKey key,
  required String dir,
  required List<KnownFeeder> feeders,
  required Future<PhoneNetwork> Function() network,
  required RoundFailures failures,
}) async {
  await Notices.sweep(dir);
  final PhoneNetwork here = await network();
  await feederRound(feeders, failures, (feeder) async {
    Future<CatchUp> attempt(Attempt attempt) => catchUpWith(
      dir: dir,
      key: key,
      identity: feeder.identity,
      manual: feeder.manualAddress,
      known: feeder.address,
      attempt: attempt,
      network: here,
    );
    CatchUp done = await attempt(const Attempt.first());
    if (done.end == CatchUpEnd.unanswered) {
      // A lease may have moved it. An owner's address is tried as it is.
      final String? located = feeder.manualAddress == null
          ? await Discovery.locate(feeder.identity, _locateWait)
          : null;
      done = await attempt(Attempt.last(located: located));
    }
    await Notices.deliver(dir: dir, identity: feeder.identity, name: feeder.name, changes: done.changes);
  });
}

/// Run `one` for each of `feeders` in turn. One that throws (the bridge's
/// error, or its panic) is logged and the round goes on: another feeder's
/// alerts must not wait on it. Its changes stay unconfirmed, so the next
/// round hands them over again. With [RoundFailures.thrown], the first
/// failure is thrown once every feeder has had its turn, so a check the
/// owner asked for never reads as a success.
Future<void> feederRound(
  List<KnownFeeder> feeders,
  RoundFailures failures,
  Future<void> Function(KnownFeeder feeder) one,
) async {
  (Object, StackTrace)? first;
  for (final KnownFeeder feeder in feeders) {
    try {
      await one(feeder);
    } on Object catch (error, stack) {
      debugPrint('the catch-up with ${feeder.name} failed this round: $error');
      first ??= (error, stack);
    }
  }
  if (failures == RoundFailures.thrown) {
    if (first case (final Object error, final StackTrace stack)) Error.throwWithStackTrace(error, stack);
  }
}

/// The app engine's channel to the Kotlin side of alerts.
const alertsChannel = MethodChannel('io.github.isaacduarte.wellfed/alerts');

/// [AlertPorts] on Android, as this phone's `_key`.
class AppAlertPorts implements AlertPorts {
  /// Ports for the phone with `_key`.
  const AppAlertPorts(this._key);

  final ClientKey _key;

  static Future<String> _dir() async => (await getApplicationSupportDirectory()).path;

  @override
  Future<FeederAlerts> alerts(String identity) async => feederAlerts(dir: await _dir(), identity: identity);

  @override
  Future<void> check(List<KnownFeeder> feeders) async {
    final String dir = await _dir();
    await catchUpRound(
      key: _key,
      dir: dir,
      feeders: feeders,
      network: () async => phoneNetwork(await alertsChannel.invokeMapMethod<Object?, Object?>('network')),
      failures: RoundFailures.thrown,
    );
  }

  @override
  Future<void> dismiss(KnownFeeder feeder, NoticeView notice) async {
    final String dir = await _dir();
    final List<NoticeChange> changes = await dismissNotice(dir: dir, identity: feeder.identity, id: notice.id);
    await Notices.deliver(dir: dir, identity: feeder.identity, name: feeder.name, changes: changes);
  }

  @override
  Future<AlertSettingsView> settings() async => alertSettings(dir: await _dir());

  @override
  Future<void> mute(MuteTarget target, {required bool muted, required List<KnownFeeder> feeders}) async {
    final String dir = await _dir();
    for (final FeederNotices each in await setMute(dir: dir, mute: target, muted: muted)) {
      final String name = feeders.where((feeder) => feeder.identity == each.identity).firstOrNull?.name ?? 'Feeder';
      await Notices.deliver(dir: dir, identity: each.identity, name: name, changes: each.changes);
    }
  }

  @override
  Future<bool> setInstant({required bool on}) async {
    final bool went = await alertsChannel.invokeMethod<bool>('instant', on) ?? false;
    await saveInstant(dir: await _dir(), enabled: on && went);
    return went;
  }

  @override
  Future<bool> instantRunning() async => await alertsChannel.invokeMethod<bool>('instantRunning') ?? false;

  @override
  Future<bool> resumeInstant() async => await alertsChannel.invokeMethod<bool>('instant', true) ?? false;

  @override
  Future<bool> notificationsAllowed() => Notices.allowed();

  @override
  Future<NotificationAsk> notificationAsk() => _notificationAsk();

  @override
  Future<bool> askForNotifications(List<KnownFeeder> feeders) async {
    switch (await _notificationAsk()) {
      case NotificationAsk.openSettings:
        // Back from them, the screens read Android's word again.
        await alertsChannel.invokeMethod<void>('notificationSettings');
        return false;
      case NotificationAsk.request:
        return _request(feeders);
    }
  }

  @override
  Future<void> postOpen(List<KnownFeeder> feeders) => postPending(feeders);

  @override
  bool get asking => _asking;
}

/// Whether an ask is out, in this engine.
bool _asking = false;

Future<T> _asked<T>(Future<T> Function() ask) async {
  _asking = true;
  try {
    return await ask();
  } finally {
    _asking = false;
  }
}

/// What the alert screens' button does while notifications are off.
enum NotificationAsk {
  /// "Allow notifications": Android's own request.
  request,

  /// "Open notification settings": the app's page in Android's settings.
  openSettings,
}

/// Ask Android's dialog only while it can still show: from Android 13, and
/// until a request on this install was refused. Past a refusal the dialog may
/// never show again, and nothing tells the app whether it would, so it opens
/// the settings instead. Below Android 13 there is no request at all.
NotificationAsk notificationAskFor({required bool sdkAtLeast33, required bool refusedBefore}) =>
    sdkAtLeast33 && !refusedBefore ? NotificationAsk.request : NotificationAsk.openSettings;

Future<NotificationAsk> _notificationAsk() async => notificationAskFor(
  sdkAtLeast33: (await alertsChannel.invokeMethod<int>('sdk') ?? 0) >= 33,
  refusedBefore: await notificationsRefused(dir: await AppAlertPorts._dir()),
);

/// Android's request, then what it answered: the open alerts post once it
/// allows, and a refusal is noted for good. `null`, Android could not ask
/// now, notes nothing.
Future<bool> _request(List<KnownFeeder> feeders) => _asked(() async {
  switch (await Notices.ask()) {
    case true:
      await postPending(feeders);
      return true;
    case false:
      await noteNotificationsRefused(dir: await AppAlertPorts._dir());
      return false;
    case null:
      return Notices.allowed();
  }
});

/// Post every change still to make for `feeders`, with no catch-up. A
/// feeder not in `feeders` keeps its own for its next catch-up.
Future<void> postPending(List<KnownFeeder> feeders) async {
  final String dir = await AppAlertPorts._dir();
  for (final FeederNotices each in await pendingNotices(dir: dir)) {
    final KnownFeeder? feeder = feeders.where((feeder) => feeder.identity == each.identity).firstOrNull;
    if (feeder == null) continue;
    await Notices.deliver(dir: dir, identity: each.identity, name: feeder.name, changes: each.changes);
  }
}

/// Ask Android once, unprompted, to let alerts through: when this phone first
/// has a feeder of its own to hear from. After that only the owner asks, from
/// the alert screens. Whether the offer is settled: `false` with no feeder of
/// the phone's own yet, or when Android could not ask now, so Home offers it
/// again later. Whatever fails here leaves the app as it was.
Future<bool> offerNotifications(List<KnownFeeder> feeders) async {
  if (feeders.isEmpty) return false;
  try {
    if (await Notices.allowed()) return true;
    final String dir = await AppAlertPorts._dir();
    if (!await notificationsOfferDue(dir: dir)) return true;
    if (await _notificationAsk() != NotificationAsk.request) return true;
    // `null`: Android could not ask now, so the offer stays due.
    final bool? allowed = await _asked(Notices.ask);
    if (allowed == null) return false;
    await notificationsOffered(dir: dir);
    if (allowed) {
      await postPending(feeders);
    } else {
      await noteNotificationsRefused(dir: dir);
    }
  } on PlatformException catch (error) {
    debugPrint('notifications were not asked for: ${error.message ?? error.code}');
  } on BridgeError catch (error) {
    debugPrint('notifications were not asked for: ${error.message}');
  }
  return true;
}

/// Set alerts going as the app opens: Notices ready, the periodic catch-up
/// asked for, and "instant while home" as the owner left it. Android lets
/// only a visible app start a foreground service, so this is where instant
/// mode comes back after a restart.
///
/// The app opens whatever becomes of this: a phone that refuses goes without
/// alerts, and the settings page says what it can.
Future<void> startAlerts() async {
  try {
    await Notices.init();
    await Notices.sweep(await AppAlertPorts._dir());
    await alertsChannel.invokeMethod<void>('schedule');
    final AlertSettingsView settings = await alertSettings(dir: await AppAlertPorts._dir());
    if (settings.instant) await alertsChannel.invokeMethod<bool>('instant', true);
  } on PlatformException catch (error) {
    debugPrint('alerts did not start: ${error.message ?? error.code}');
  } on BridgeError catch (error) {
    debugPrint('alerts did not start: ${error.message}');
  }
}

/// This phone forgot the feeder `identity`: its alerts go too, and their
/// notifications come off.
Future<void> forgetFeederAlerts(String identity) async {
  final Int32List ids = await forgetAlerts(dir: await AppAlertPorts._dir(), identity: identity);
  await Notices.remove(ids);
}

/// The process holds of this app's engine. The screen stays on with them,
/// as a locked phone dozes whatever the service asks: a run's own sockets and
/// discovery stall until someone wakes it.
final runHolds = RunHolds((text) async {
  await _screen.invokeMethod<void>('keepOn', text != null);
  return await alertsChannel.invokeMethod<bool>('hold', text) ?? false;
}, (run, {required held}) => holdAlerts(run: run, held: held));

const _screen = MethodChannel('io.github.isaacduarte.wellfed/screen');
