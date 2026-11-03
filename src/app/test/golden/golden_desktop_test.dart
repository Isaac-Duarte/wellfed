import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/desktop/sidebar_shell.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/screens/home/add_feeder.dart';
import 'package:wellfed/screens/home/home.dart';
import 'package:wellfed/screens/home/welcome.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/settings/backups.dart';

import '../feeder/feeder_support.dart';
import '../support/fakes.dart';
import '../support/harness.dart';
import 'golden_fixtures.dart';
import 'golden_support.dart';

const _desktopSize = Size(1280, 800);

/// A golden of the desktop build: the platform set to Linux for the test
/// alone, as the theme and every layout read it while they build.
void _desktopGolden(String name, Future<void> Function(WidgetTester tester) body) => testWidgets(
  'desktop $name',
  skip: skipGoldens,
  (tester) => atFixtureNow(() async {
    debugDefaultTargetPlatformOverride = TargetPlatform.linux;
    try {
      await body(tester);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  }),
);

/// `page` in the desktop window beside the sidebar, at `at`.
Widget _window(FeederFleet fleet, Place? at, Widget page) =>
    SidebarShell(fleet: fleet, at: at, go: (_) {}, back: () {}, child: page);

Future<void> _desktop(WidgetTester tester, Widget screen, Brightness brightness) async {
  frame(tester, size: _desktopSize);
  await tester.pumpWidget(themed(screen, brightness: brightness));
  await settle(tester);
}

/// The fixture's feeder in a fleet, as the sidebar lists it.
Future<FeederFleet> _fleet(WidgetTester tester, {Card? card}) async {
  final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
  answerAsFull(session);
  session.status(card ?? fullCard());
  await tester.pump();
  return fleet;
}

void main() {
  setUpGoldens('golden_desktop_test.dart');

  for (final Brightness brightness in Brightness.values) {
    group('${brightness.name} desktop', () {
      _desktopGolden('welcome', (tester) async {
        final fleet = FeederFleet(FakeBook(), FakeFeederPorts());
        addTearDown(fleet.dispose);
        await _desktop(tester, _window(fleet, const OverviewPlace(), const WelcomeScreen()), brightness);
        await matchesGolden('desktop_welcome', brightness);
      });

      _desktopGolden('overview', (tester) async {
        final FeederFleet fleet = await _fleet(tester, card: fourMealsCard());
        await _desktop(
          tester,
          _window(
            fleet,
            const OverviewPlace(),
            HomeScreen(fleet: fleet, earlierInstall: () async => PendingInstall.nothing),
          ),
          brightness,
        );
        await decodeImages(tester);
        await matchesGolden('desktop_overview', brightness);
      });

      for (final (String name, int tab) in [
        ('today', todayTab),
        ('schedule', scheduleTab),
        ('settings', settingsTab),
      ]) {
        _desktopGolden('feeder $name', (tester) async {
          final FeederFleet fleet = await _fleet(tester, card: name == 'schedule' ? fourMealsCard() : null);
          final FeederController feeder = fleet.all.single;
          await _desktop(
            tester,
            _window(fleet, FeederPlace(feeder.state.known.identity), shell(feeder, tab)),
            brightness,
          );
          await decodeImages(tester);
          await matchesGolden('desktop_$name', brightness);
        });
      }

      _desktopGolden('adding a feeder', (tester) async {
        final FeederFleet fleet = await _fleet(tester);
        await _desktop(tester, _window(fleet, const AddPlace(), const AddFeederScreen()), brightness);
        await matchesGolden('desktop_add_feeder', brightness);
      });

      _desktopGolden('app settings', (tester) async {
        final FeederFleet fleet = await _fleet(tester);
        final book = FakeBook()..label = 'My computer';
        await _desktop(
          tester,
          _window(
            fleet,
            const SettingsPlace(),
            AppSettingsScreen(book: book, newBackups: () => BackupsFlow(book, FakeBackupPorts())),
          ),
          brightness,
        );
        await matchesGolden('desktop_app_settings', brightness);
      });

      _desktopGolden('the feed dialog', (tester) async {
        final FeederFleet fleet = await _fleet(tester);
        final FeederController feeder = fleet.all.single;
        await _desktop(
          tester,
          _window(fleet, FeederPlace(feeder.state.known.identity), shell(feeder, todayTab)),
          brightness,
        );
        await tester.tap(find.text('Feed now').first);
        await settle(tester);
        await matchesGolden('desktop_feed_dialog', brightness);
      });
    });
  }
}
