import 'dart:io';

import 'package:flutter/material.dart' hide Card;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/feed_sheet.dart';
import 'package:wellfed/screens/feeder/schedule.dart';
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/screens/feeder/today.dart';
import 'package:wellfed/screens/home/feeder_card.dart';
import 'package:wellfed/screens/home/welcome.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/screens/setup/pairing_view.dart';
import 'package:wellfed/screens/setup/setup_screen.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/pairing/pairing.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/state/setup/setup.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

import '../install/install_support.dart';
import '../restore/restore_support.dart';
import '../support/fake_install.dart';
import '../support/fakes.dart';
import '../support/harness.dart';

/// Pixels differ between font rasterizers, so one platform owns these files.
final _skip = !Platform.isLinux;

/// The app's own fonts: the test default draws every letter as a box.
Future<void> _loadFonts() async {
  final families = {
    Fonts.family: ['Inter-Regular', 'Inter-Medium', 'Inter-SemiBold', 'Inter-ExtraBold'],
    Fonts.mono: ['JetBrainsMono-Regular'],
  };
  for (final MapEntry(key: family, value: files) in families.entries) {
    final loader = FontLoader(family);
    for (final file in files) {
      loader.addFont(rootBundle.load('assets/fonts/$file.ttf'));
    }
    await loader.load();
  }
  await (FontLoader('MaterialIcons')..addFont(rootBundle.load('fonts/MaterialIcons-Regular.otf'))).load();
}

Future<void> _phone(WidgetTester tester, Widget screen, Brightness brightness) async {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(themed(screen, brightness: brightness));
  await settle(tester);
}

Future<FeederController> _feeder(WidgetTester tester) async {
  final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
  session.status(fullCard());
  await tester.pump();
  return feeder;
}

Future<void> _matches(String name, Brightness brightness) =>
    expectLater(find.byType(MaterialApp), matchesGoldenFile('files/$name.${brightness.name}.png'));

PairingView _pairing(PairingState state) => PairingView(
  flow: PairingFlow(FakeBook(), FakePairingPorts())..show(state),
  ble: true,
  onRetry: () {},
  onNewCode: () {},
);

void main() {
  setUpAll(_loadFonts);

  for (final Brightness brightness in Brightness.values) {
    group(brightness.name, () {
      testWidgets('the shared widgets', skip: _skip, (tester) async {
        await _phone(
          tester,
          Scaffold(
            body: ListView(
              padding: const EdgeInsets.all(Space.s4),
              children: const [
                Headline('A headline'),
                SizedBox(height: Space.s3),
                Bar(value: 0.4),
                SizedBox(height: Space.s3),
                Row(
                  children: [
                    StatusChip('Ready', color: Color(0xFF2E7D32)),
                    SizedBox(width: Space.s3),
                    FoodDot(present: true),
                    SizedBox(width: Space.s3),
                    FoodDot(present: false),
                  ],
                ),
                SizedBox(height: Space.s3),
                Notice(tone: Tone.neutral, glyph: Glyph.info, text: 'A neutral note.'),
                SizedBox(height: Space.s2),
                Notice(tone: Tone.warning, glyph: Glyph.warning, lead: 'A warning.', text: 'It says what to do.'),
                SizedBox(height: Space.s2),
                Notice(tone: Tone.fault, lead: 'A fault.', text: 'It says what went wrong.'),
                SizedBox(height: Space.s3),
                NumberedSteps(['Hold **Lock** for 5 seconds.', 'Let go.']),
                SizedBox(height: Space.s3),
                SectionLabel('A section'),
                SizedBox(height: Space.s2),
                FactList([('A fact', Text('Its value'))]),
                SizedBox(height: Space.s3),
                Searching('Looking for feeders…'),
              ],
            ),
          ),
          brightness,
        );
        await _matches('shared_widgets', brightness);
      });

      testWidgets('the welcome', skip: _skip, (tester) async {
        await _phone(tester, const WelcomeScreen(), brightness);
        await _matches('welcome', brightness);
      });

      testWidgets('a feeder card', skip: _skip, (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, Scaffold(body: FeederCard(controller: feeder)), brightness);
        await _matches('feeder_card', brightness);
      });

      testWidgets('today', skip: _skip, (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, Scaffold(body: TodayTab(feeder: feeder)), brightness);
        await _matches('today', brightness);
      });

      testWidgets('the schedule', skip: _skip, (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, Scaffold(body: ScheduleTab(feeder: feeder)), brightness);
        await _matches('schedule', brightness);
      });

      testWidgets('feeder settings', skip: _skip, (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, Scaffold(body: SettingsTab(feeder: feeder)), brightness);
        await _matches('feeder_settings', brightness);
      });

      testWidgets('the feed sheet', skip: _skip, (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(
          tester,
          Builder(
            builder: (context) => Scaffold(
              body: Center(
                child: TextButton(onPressed: () => showFeedSheet(context, feeder), child: const Text('Open')),
              ),
            ),
          ),
          brightness,
        );
        await tester.tap(find.text('Open'));
        await settle(tester);
        await _matches('feed_sheet', brightness);
      });

      testWidgets('setup', skip: _skip, (tester) async {
        await _phone(
          tester,
          SetupScreen(newFlow: () => SetupFlow(FakeBook(), FakePairingPorts(), FakePhonePorts())),
          brightness,
        );
        await _matches('setup', brightness);
      });

      testWidgets('a failed pairing', skip: _skip, (tester) async {
        await _phone(
          tester,
          _pairing(const PairingFailed(failure: PairingFailure.timeout, feederName: 'Kitchen')),
          brightness,
        );
        await _matches('pairing_failed', brightness);
      });

      testWidgets('app settings', skip: _skip, (tester) async {
        final book = FakeBook()..label = 'My phone';
        await _phone(
          tester,
          AppSettingsScreen(book: book, newBackups: () => BackupsFlow(book, FakeBackupPorts())),
          brightness,
        );
        await _matches('app_settings', brightness);
      });

      testWidgets('the install', skip: _skip, (tester) async {
        tester.view.physicalSize = const Size(412, 915);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        await tester.pumpWidget(installApp(FakeBook(), FakeInstallPorts(), brightness: brightness));
        await settle(tester);
        await _matches('install', brightness);
      });

      testWidgets('the restore', skip: _skip, (tester) async {
        await restoreOpen(tester, FakeRestorePorts(), brightness: brightness);
        await tester.binding.setSurfaceSize(null);
        await _matches('restore', brightness);
      });
    });
  }
}
