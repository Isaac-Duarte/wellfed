import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/screens/alerts/alert_settings_screen.dart';
import 'package:wellfed/screens/alerts/alerts_screen.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/alerts/alert_settings.dart';
import 'package:wellfed/state/alerts/alert_words.dart';
import 'package:wellfed/state/alerts/alerts.dart';
import 'package:wellfed/widgets/layout.dart';

import '../support/accessibility.dart';
import '../support/fakes.dart';
import 'alerts_support.dart';

Future<void> _showAlerts(WidgetTester tester, FakeAlertPorts ports, {Brightness brightness = Brightness.light}) async {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    themed(
      AlertsScreen(name: 'Kitchen', newController: () => FeederAlertsController(ports, kitchen)),
      brightness: brightness,
    ),
  );
  await settle(tester);
}

Future<void> _showSettings(
  WidgetTester tester,
  FakeAlertPorts ports, {
  Brightness brightness = Brightness.light,
}) async {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    themed(
      AlertSettingsScreen(newController: () => AlertSettingsController(ports, FakeBook()..feeders = [kitchen])),
      brightness: brightness,
    ),
  );
  await settle(tester);
}

void main() {
  test('home is told by its gateway and subnet, not by DNS that drifts under it', () {
    final Map<Object?, Object?> home = {'gateway': '192.168.1.1', 'subnet': '192.168.1.0/24'};
    expect(phoneNetwork(home), const PhoneNetwork.wifi(gateway: '192.168.1.1', subnet: '192.168.1.0/24'));
    expect(
      phoneNetwork({
        ...home,
        'dns': ['192.168.1.1', '2001:db8::1'],
        'domains': 'lan',
      }),
      phoneNetwork(home),
      reason: 'a rotated IPv6 resolver is still home',
    );
    expect(phoneNetwork(null), const PhoneNetwork.elsewhere());
  });

  group("a feeder's alerts", () {
    test('load, check and dismiss reach the ports and list again', () async {
      final ports = FakeAlertPorts()..notices = [failedMeal(), foodLow()];
      final alerts = FeederAlertsController(ports, kitchen);
      addTearDown(alerts.dispose);

      await alerts.load();
      expect((alerts.state as AlertsShown).notices, hasLength(2));
      expect((alerts.state as AlertsShown).reachedMs, isNull);

      await alerts.check();
      expect(ports.checked.single.single.identity, 'ab', reason: 'this feeder alone');
      expect((alerts.state as AlertsShown).reachedMs, isNotNull);

      await alerts.dismiss(failedMeal());
      expect(ports.dismissed, [0x40000001]);
      expect((alerts.state as AlertsShown).notices.map((notice) => notice.kind), [NoticeKind.foodLow]);
    });

    test("a check whose round failed for the feeder says so, and isn't taken for a success", () async {
      final ports = _RoundPorts()..notices = [foodLow()];
      final alerts = FeederAlertsController(ports, kitchen);
      addTearDown(alerts.dispose);
      await alerts.load();

      await alerts.check();
      final shown = alerts.state as AlertsShown;
      expect(shown.checking, isFalse);
      expect(shown.problem, "Couldn't check: no route to the feeder.");
      expect(shown.reachedMs, isNull, reason: 'the list was not read as fresh');
    });

    test('a store that does not read is a failure, and a failed check keeps the list', () async {
      final ports = FakeAlertPorts()..fails = const BridgeError(kind: ErrorKind.store, message: 'disk');
      final alerts = FeederAlertsController(ports, kitchen);
      addTearDown(alerts.dispose);
      await alerts.load();
      expect(alerts.state, isA<AlertsFailed>());

      ports.fails = null;
      ports.notices = [foodLow()];
      await alerts.load();
      ports.fails = const BridgeError(kind: ErrorKind.store, message: 'disk');
      await alerts.check();
      final shown = alerts.state as AlertsShown;
      expect(shown.notices, hasLength(1));
      expect(shown.checking, isFalse);
      expect(shown.problem, contains('disk'));
    });

    test('anything else thrown still stops the spinner', () async {
      final ports = FakeAlertPorts()..notices = [foodLow()];
      final alerts = FeederAlertsController(ports, kitchen);
      addTearDown(alerts.dispose);
      await alerts.load();

      ports.fails = StateError('a bridge panic');
      await alerts.check();
      final shown = alerts.state as AlertsShown;
      expect(shown.checking, isFalse);
      expect(shown.problem, contains('a bridge panic'));
      expect(shown.notices, hasLength(1));
    });

    testWidgets('the screen words each alert, dismisses one and checks', (tester) async {
      final ports = FakeAlertPorts()..notices = [failedMeal(), foodLow()];
      await _showAlerts(tester, ports);

      expect(find.text('Kitchen: a meal failed'), findsOneWidget);
      expect(find.textContaining('1 of 2 portions dropped. The wheel jammed.'), findsOneWidget);
      expect(find.text('Kitchen is low on food'), findsOneWidget);
      expect(find.textContaining('not notified'), findsOneWidget, reason: 'the muted one');
      expect(find.textContaining('Not checked yet'), findsOneWidget);

      await tester.tap(find.byTooltip('Dismiss on this phone').first);
      await settle(tester);
      expect(find.text('Kitchen: a meal failed'), findsNothing);

      await tester.tap(find.text('Check now'));
      await settle(tester);
      expect(find.textContaining('Last checked'), findsOneWidget);
    });

    testWidgets("notifications that are off say so, and allowing them posts this feeder's", (tester) async {
      final ports = FakeAlertPorts()
        ..notices = [failedMeal()]
        ..allowed = false;
      await _showAlerts(tester, ports);

      expect(find.textContaining('Notifications are off.', findRichText: true), findsOneWidget);
      await tester.tap(find.text('Allow notifications'));
      await settle(tester);
      expect(ports.asked.single.single.identity, 'ab', reason: "this feeder's open alerts post");
      expect(find.textContaining('Notifications are off.', findRichText: true), findsNothing);
    });

    testWidgets("back from Android's settings with notifications on, the notice goes and alerts post", (tester) async {
      final ports = FakeAlertPorts()
        ..notices = [failedMeal()]
        ..allowed = false;
      await _showAlerts(tester, ports);
      expect(find.textContaining('Notifications are off.', findRichText: true), findsOneWidget);

      ports.allowed = true;
      [
        AppLifecycleState.inactive,
        AppLifecycleState.hidden,
        AppLifecycleState.paused,
        AppLifecycleState.hidden,
        AppLifecycleState.inactive,
        AppLifecycleState.resumed,
      ].forEach(tester.binding.handleAppLifecycleStateChanged);
      await settle(tester);
      expect(find.textContaining('Notifications are off.', findRichText: true), findsNothing);
      expect(ports.postedOpen.single.single.identity, 'ab');
    });

    testWidgets('a refused request turns the button to the settings, which it opens without asking', (tester) async {
      final ports = FakeAlertPorts()
        ..notices = [failedMeal()]
        ..allowed = false
        ..grants = false;
      await _showAlerts(tester, ports);

      await tester.tap(find.text('Allow notifications'));
      await settle(tester);
      expect(ports.asked, hasLength(1));
      expect(ports.openedSettings, 0, reason: "the owner's own answer, left where it was");
      expect(find.textContaining('Notifications are off.', findRichText: true), findsOneWidget);

      await tester.tap(find.text('Open notification settings'));
      await settle(tester);
      expect(ports.openedSettings, 1);
      expect(ports.asked, hasLength(1), reason: 'never asks again');
    });

    testWidgets('nothing wrong, and alerts too new for this app, say so', (tester) async {
      final ports = FakeAlertPorts()..unread = 2;
      await _showAlerts(tester, ports);
      expect(find.text('Nothing is wrong that Kitchen has said.'), findsOneWidget);
      expect(find.textContaining('2 more alerts this app is too old to read'), findsOneWidget);
    });

    for (final Brightness brightness in Brightness.values) {
      testWidgets('the alert list is accessible in the ${brightness.name} theme', (tester) async {
        await _showAlerts(tester, FakeAlertPorts()..notices = [failedMeal(), foodLow()], brightness: brightness);
        await expectAccessible(tester);
      });
    }
  });

  group('alert settings', () {
    test('mutes by kind and level, and instant mode, reach the ports', () async {
      final ports = FakeAlertPorts();
      final settings = AlertSettingsController(ports, FakeBook());
      addTearDown(settings.dispose);
      await settings.load();

      await settings.muteLevel(AlertLevel.security, muted: true);
      await settings.muteKind(NoticeKind.foodLow, muted: true);
      final shown = settings.state as AlertSettingsShown;
      expect(shown.mutedLevels, {AlertLevel.security});
      expect(shown.muted(kinds[1]), isTrue, reason: 'muted by kind');
      expect(shown.muted(kinds[2]), isTrue, reason: 'muted by its level');
      expect(shown.muted(kinds[0]), isFalse);

      await settings.setInstant(on: true);
      expect(
        (settings.state as AlertSettingsShown).watching,
        isA<PhoneWatching>().having((w) => w.instant, 'instant', isTrue),
      );
    });

    test('instant mode Android refuses stays off and says so', () async {
      final ports = FakeAlertPorts()..instantRefused = true;
      final settings = AlertSettingsController(ports, FakeBook());
      addTearDown(settings.dispose);
      await settings.load();
      await settings.setInstant(on: true);
      final shown = settings.state as AlertSettingsShown;
      expect(shown.watching, isA<PhoneWatching>().having((w) => w.instant, 'instant', isFalse));
      expect(shown.problem, contains("Android didn't let it start"));
    });

    test('back in front with notifications turned on, the open alerts post', () async {
      final ports = FakeAlertPorts()..allowed = false;
      final settings = AlertSettingsController(ports, FakeBook()..feeders = [kitchen]);
      addTearDown(settings.dispose);
      await settings.load();

      await settings.resumed();
      expect(ports.postedOpen, isEmpty, reason: 'still off');
      ports.allowed = true;
      await settings.resumed();
      expect(ports.postedOpen.single.single.identity, 'ab');
      expect((settings.state as AlertSettingsShown).notificationsAllowed, isTrue);
      await settings.resumed();
      expect(ports.postedOpen, hasLength(1), reason: 'only when they were just turned on');
    });

    test("an ask's own dialog resuming the app posts nothing twice", () async {
      final ports = FakeAlertPorts()..allowed = false;
      final settings = AlertSettingsController(ports, FakeBook()..feeders = [kitchen]);
      addTearDown(settings.dispose);
      await settings.load();

      ports
        ..allowed = true
        ..asking = true;
      await settings.resumed();
      expect(ports.postedOpen, isEmpty, reason: 'the ask posts for itself');

      final alerts = FeederAlertsController(ports, kitchen);
      addTearDown(alerts.dispose);
      ports.allowed = false;
      await alerts.load();
      ports.allowed = true;
      await alerts.resumed();
      expect(ports.postedOpen, isEmpty);
    });

    test('instant mode on but stopped starts again, and says so when Android refuses', () async {
      final ports = FakeAlertPorts()..instant = true;
      final settings = AlertSettingsController(ports, FakeBook());
      addTearDown(settings.dispose);
      await settings.load();
      expect(
        (settings.state as AlertSettingsShown).watching,
        isA<PhoneWatching>().having((w) => w.running, 'running', isTrue),
      );
      expect(ports.instant, isTrue);

      ports
        ..running = false
        ..resumes = false;
      await settings.load();
      final shown = settings.state as AlertSettingsShown;
      final watching = shown.watching as PhoneWatching;
      expect(watching.instant, isTrue, reason: 'still on in the settings');
      expect(watching.running, isFalse);
    });

    testWidgets('instant mode that is not running says when it comes back', (tester) async {
      final ports = FakeAlertPorts()
        ..instant = true
        ..resumes = false;
      await _showSettings(tester, ports);
      expect(find.textContaining("Instant isn't running.", findRichText: true), findsOneWidget);
      expect(find.textContaining('turn instant off and on here', findRichText: true), findsOneWidget);
      final Finder notice = find.ancestor(
        of: find.textContaining("Instant isn't running.", findRichText: true),
        matching: find.byType(Notice),
      );
      expect(
        find.descendant(of: notice, matching: find.textContaining('open Wellfed', findRichText: true)),
        findsNothing,
        reason: 'the app is open',
      );
    });

    testWidgets('the page says alerts work at home only, and asks for notifications', (tester) async {
      final ports = FakeAlertPorts()..allowed = false;
      await _showSettings(tester, ports);

      expect(find.textContaining('Alerts work at home.', findRichText: true), findsOneWidget);
      expect(find.textContaining("you won't get alerts away from home", findRichText: true), findsOneWidget);
      await tester.tap(find.text('Allow notifications'));
      await settle(tester);
      expect(find.text('Allow notifications'), findsNothing);
      expect(ports.asked.single.single.identity, 'ab', reason: 'the open alerts post');

      await tester.tap(find.text('Instant while home'));
      await settle(tester);
      expect(ports.instant, isTrue);

      final Finder security = find.widgetWithText(SwitchListTile, levelName(AlertLevel.security));
      await tester.ensureVisible(security);
      await settle(tester);
      await tester.tap(security);
      await settle(tester);
      expect(ports.mutedLevels, {AlertLevel.security});
      await tester.scrollUntilVisible(find.text('Off with Security'), 100);
      expect(find.text('Off with Security'), findsOneWidget);
    });

    testWidgets("below Android 13, or once refused, the settings page opens Android's settings", (tester) async {
      final ports = FakeAlertPorts()
        ..allowed = false
        ..ask = NotificationAsk.openSettings;
      await _showSettings(tester, ports);
      expect(find.text('Allow notifications'), findsNothing);
      await tester.tap(find.text('Open notification settings'));
      await settle(tester);
      expect(ports.openedSettings, 1);
      expect(ports.asked, isEmpty);
    });

    for (final Brightness brightness in Brightness.values) {
      testWidgets('the settings page is accessible in the ${brightness.name} theme', (tester) async {
        await _showSettings(tester, FakeAlertPorts(), brightness: brightness);
        await expectAccessible(tester);
      });
    }
  });

  test('every kind of notice has its words', () {
    for (final NoticeKind kind in NoticeKind.values) {
      final NoticeWords words = noticeWords(
        NoticeView(
          id: 1,
          kind: kind,
          level: AlertLevel.soon,
          seenMs: 0,
          detail: const NoticeDetail.plain(),
          quiet: false,
        ),
        'Kitchen',
      );
      expect(words.title, contains('Kitchen'), reason: '$kind');
      expect(words.body, isNotEmpty, reason: '$kind');
      expect(kindName(kind), isNotEmpty);
    }
  });
}

/// Ports whose check runs the app's own round over the feeders, each of
/// whose catch-ups fails as the bridge would.
class _RoundPorts extends FakeAlertPorts {
  @override
  Future<void> check(List<KnownFeeder> feeders) => feederRound(
    feeders,
    RoundFailures.thrown,
    (feeder) async => throw const BridgeError(kind: ErrorKind.closed, message: 'no route to the feeder'),
  );
}
