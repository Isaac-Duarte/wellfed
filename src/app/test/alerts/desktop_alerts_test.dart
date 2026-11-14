import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/platform/alert_watch.dart';
import 'package:wellfed/screens/alerts/alert_settings_screen.dart';
import 'package:wellfed/screens/alerts/alerts_screen.dart';
import 'package:wellfed/state/alerts/alert_settings.dart';
import 'package:wellfed/state/alerts/alert_words.dart';
import 'package:wellfed/state/alerts/alerts.dart';

import '../support/fakes.dart';
import 'alerts_support.dart';

/// `body` with the layout and wording of a computer, as `isDesktop` reads
/// the target platform.
Future<void> _onComputer(Future<void> Function() body) async {
  debugDefaultTargetPlatformOverride = TargetPlatform.linux;
  try {
    await body();
  } finally {
    debugDefaultTargetPlatformOverride = null;
  }
}

Future<void> _show(WidgetTester tester, Widget screen) async {
  tester.view.physicalSize = const Size(1280, 1600);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(themed(screen));
  await settle(tester);
}

AlertSettingsScreen _settings(FakeAlertPorts ports) =>
    AlertSettingsScreen(newController: () => AlertSettingsController(ports, FakeBook()..feeders = [kitchen]));

void main() {
  group("a computer's watch", () {
    testWidgets('rounds at once, then each minute, until it stops', (tester) async {
      var rounds = 0;
      final watch = AlertWatch(() async => rounds++)..start();
      await tester.pump();
      expect(rounds, 1, reason: 'the app just opened');

      await tester.pump(computerRoundEvery);
      expect(rounds, 2);
      await tester.pump(computerRoundEvery * 3);
      expect(rounds, 5);

      watch.stop();
      await tester.pump(computerRoundEvery * 2);
      expect(rounds, 5, reason: 'closing the app ends the checks');
    });

    testWidgets('a slow round makes the next minute skip rather than pile up', (tester) async {
      var started = 0;
      final slow = Completer<void>();
      final watch = AlertWatch(() {
        started++;
        return slow.future;
      })..start();
      await tester.pump();

      await tester.pump(computerRoundEvery * 3);
      expect(started, 1, reason: 'one round at a time');

      slow.complete();
      await tester.pump(computerRoundEvery);
      expect(started, 2);
      watch.stop();
    });

    testWidgets('a round that fails leaves the next its turn', (tester) async {
      var rounds = 0;
      final watch = AlertWatch(() async {
        rounds++;
        throw StateError('the feeder went away mid-read');
      })..start();
      await tester.pump();

      await tester.pump(computerRoundEvery);
      expect(rounds, 2);
      watch.stop();
    });
  });

  group("a computer's settings", () {
    test('read as checks while open, with no instant mode asked about', () async {
      final ports = FakeAlertPorts()
        ..whileOpen = true
        ..instant = true;
      final settings = AlertSettingsController(ports, FakeBook());
      addTearDown(settings.dispose);
      await settings.load();

      expect((settings.state as AlertSettingsShown).watching, isA<OpenWatching>());
      expect(ports.running, isFalse, reason: 'nothing asked Android to start instant mode');
    });

    testWidgets('say alerts come while Wellfed is open, and offer no instant switch', (tester) async {
      await _onComputer(() async {
        await _show(tester, _settings(FakeAlertPorts()..whileOpen = true));

        expect(find.textContaining('Alerts work at home, while Wellfed is open.', findRichText: true), findsOneWidget);
        expect(find.textContaining('Closing Wellfed stops the checks'), findsOneWidget);
        expect(find.text('Every minute while open'), findsOneWidget);
        expect(find.text('Instant while home'), findsNothing);
        expect(find.text('ON THIS COMPUTER'), findsOneWidget);
      });
    });

    testWidgets('with notifications off in its settings, offer to open them', (tester) async {
      await _onComputer(() async {
        final ports = FakeAlertPorts()
          ..whileOpen = true
          ..allowed = false
          ..ask = NotificationAsk.openSettings;
        await _show(tester, _settings(ports));

        expect(find.textContaining("This computer's settings keep Wellfed's notifications off"), findsOneWidget);
        await tester.tap(find.text('Open notification settings'));
        await settle(tester);
        expect(ports.openedSettings, 1);
      });
    });

    testWidgets('with no notification server, note it and offer nothing to press', (tester) async {
      await _onComputer(() async {
        final ports = FakeAlertPorts()
          ..whileOpen = true
          ..allowed = false
          ..ask = NotificationAsk.unavailable;
        await _show(tester, _settings(ports));

        expect(find.textContaining('no notification service running', findRichText: true), findsOneWidget);
        expect(find.text('Allow notifications'), findsNothing);
        expect(find.text('Open notification settings'), findsNothing);
        expect(find.text('Every minute while open'), findsOneWidget, reason: 'the checks still run');
      });
    });
  });

  testWidgets("a feeder's alerts on a computer with no notification server list with a note", (tester) async {
    await _onComputer(() async {
      final ports = FakeAlertPorts()
        ..whileOpen = true
        ..allowed = false
        ..ask = NotificationAsk.unavailable
        ..notices = [failedMeal()];
      await _show(tester, AlertsScreen(name: 'Kitchen', newController: () => FeederAlertsController(ports, kitchen)));

      expect(find.textContaining('no notification service running', findRichText: true), findsOneWidget);
      expect(find.byType(FilledButton), findsNothing);
      expect(find.text(noticeWords(failedMeal(), 'Kitchen').title), findsOneWidget);
      expect(find.byTooltip('Dismiss on this computer'), findsOneWidget);
      expect(find.textContaining('this computer checks at home while Wellfed is open'), findsOneWidget);
    });
  });

  test('the words for notifications that are off name what keeps them off', () async {
    expect(notificationAskLabel(NotificationAsk.unavailable), isNull);
    expect(notificationsOffWords(NotificationAsk.request), contains('Android'));
    await _onComputer(() async {
      expect(notificationsOffWords(NotificationAsk.openSettings), contains("This computer's settings"));
      expect(notificationsOffWords(NotificationAsk.unavailable), contains('no notification service'));
    });
  });
}
