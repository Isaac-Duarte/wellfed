import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/feed_sheet.dart';
import 'package:wellfed/screens/feeder/meal_call.dart';
import 'package:wellfed/screens/feeder/meal_editor.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/screens/feeder/wifi.dart';
import 'package:wellfed/screens/home/add_feeder.dart';
import 'package:wellfed/screens/home/home.dart';
import 'package:wellfed/screens/home/welcome.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/screens/setup/setup_screen.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/state/setup/setup.dart';

import '../feeder/feeder_support.dart';
import '../feeder/schedule_history_support.dart';
import '../install/install_support.dart';
import '../restore/restore_support.dart';
import '../support/fake_install.dart';
import '../support/fakes.dart';
import '../support/harness.dart';

/// Android's largest font size doubles the text. Any overflow at that size
/// reports a Flutter error, which fails the test on its own.
const _largest = 2.0;

void _frame(WidgetTester tester) {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  tester.platformDispatcher.textScaleFactorTestValue = _largest;
  addTearDown(tester.view.reset);
  addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
}

Future<void> _show(WidgetTester tester, Widget screen) async {
  _frame(tester);
  await tester.pumpWidget(themed(screen));
  await settle(tester);
}

Future<FeederController> _feeder(WidgetTester tester) async {
  final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
  answerAsFull(session);
  session.status(fullCard());
  await tester.pump();
  return feeder;
}

void main() {
  testWidgets('the welcome', (tester) => _show(tester, const WelcomeScreen()));

  testWidgets('adding a feeder', (tester) => _show(tester, const AddFeederScreen()));

  testWidgets('home', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    answerAsFull(session);
    session.status(fullCard());
    await _show(tester, HomeScreen(fleet: fleet, earlierInstall: () async => PendingInstall.nothing));
  });

  for (final (String name, int tab) in [
    ('today', todayTab),
    ('the schedule', scheduleTab),
    ('feeder settings', settingsTab),
  ]) {
    testWidgets(name, (tester) async {
      final FeederController feeder = await _feeder(tester);
      await _show(tester, FeederShell(feeder: feeder, initialTab: tab));
    });
  }

  testWidgets('Wi-Fi settings, and its network picker', (tester) async {
    final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
    session.status(fullCard());
    await _show(tester, WifiScreen(feeder: feeder));
    await tester.tap(find.text('Home'));
    await settle(tester);
  });

  testWidgets('the meal call', (tester) async {
    final FeederController feeder = await _feeder(tester);
    await _show(tester, MealCallScreen(feeder: feeder, pick: () async => null));
    await tester.tap(find.byTooltip('Record on this phone'));
    await tester.pump(const Duration(seconds: 3));
    await tester.tap(find.text('Cancel'));
    await settle(tester);
  });

  testWidgets('the meal editor', (tester) async {
    final FeederController feeder = await _feeder(tester);
    await _show(tester, MealEditor(feeder: feeder, held: planPlans([planPlan(60)]), slot: 0));
  });

  testWidgets('app settings', (tester) async {
    final book = FakeBook()..label = 'My phone';
    await _show(tester, AppSettingsScreen(book: book, newBackups: () => BackupsFlow(book, FakeBackupPorts())));
  });

  testWidgets('the feed sheet', (tester) async {
    final FeederController feeder = await _feeder(tester);
    await _show(
      tester,
      Builder(
        builder: (context) => Scaffold(
          body: TextButton(onPressed: () => showFeedSheet(context, feeder), child: const Text('Open')),
        ),
      ),
    );
    await tester.tap(find.text('Open'));
    await settle(tester);
  });

  testWidgets('setup', (tester) async {
    await _show(tester, SetupScreen(newFlow: () => SetupFlow(FakeBook(), FakePairingPorts(), FakePhonePorts())));
  });

  testWidgets('the install', (tester) async {
    _frame(tester);
    await tester.pumpWidget(installApp(FakeBook(), FakeInstallPorts()));
    await settle(tester);
  });

  testWidgets('the restore', (tester) async {
    await restoreOpen(tester, FakeRestorePorts());
    _frame(tester);
    await settle(tester);
  });
}
