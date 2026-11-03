import 'dart:io';

import 'package:flutter/material.dart' hide Card;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/feed_sheet.dart';
import 'package:wellfed/screens/feeder/meal_call.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/screens/home/add_feeder.dart';
import 'package:wellfed/screens/home/feeder_card.dart';
import 'package:wellfed/screens/home/home.dart';
import 'package:wellfed/screens/home/welcome.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/screens/setup/pairing_view.dart';
import 'package:wellfed/screens/setup/setup_screen.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/pairing/pairing.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/state/setup/setup.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

import '../feeder/feeder_support.dart';
import '../install/install_support.dart';
import '../restore/restore_support.dart';
import '../support/fake_install.dart';
import '../support/fakes.dart';
import '../support/harness.dart';
import 'comparator.dart';

/// Pixels differ between font rasterizers by as much as a real change, so
/// Linux owns these files; macOS runs them in a Linux container
/// (`make check-app`). x86_64 and arm64 Linux agree within the comparator's
/// tolerance.
final bool _skip = !Platform.isLinux;

/// The app's own fonts: the test default draws every letter as a box.
Future<void> _loadFonts() async {
  final Map<String, List<String>> families = {
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

const _phoneSize = Size(412, 915);
const _smallPhone = Size(360, 740);

/// Large text, as a person who sets Android's font size up a step or two sees.
const _largeText = 1.3;

Future<void> _phone(
  WidgetTester tester,
  Widget screen,
  Brightness brightness, {
  Size size = _phoneSize,
  double text = 1,
}) async {
  _frame(tester, size: size, text: text);
  await tester.pumpWidget(themed(screen, brightness: brightness));
  await settle(tester);
}

void _frame(WidgetTester tester, {Size size = _phoneSize, double text = 1}) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  tester.platformDispatcher.textScaleFactorTestValue = text;
  addTearDown(tester.view.reset);
  addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
}

/// The fixture's feeder, its session answering as [answerAsFull] does.
Future<FeederController> _feeder(WidgetTester tester, {Card? card}) async {
  final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
  answerAsFull(session);
  session.status(card ?? fullCard());
  await tester.pump();
  return feeder;
}

/// Decode the screen's photos for real: the fake clock never lets a decode
/// finish, and a photo no earlier test cached would draw as nothing.
Future<void> _decodeImages(WidgetTester tester) async {
  for (final Element element in find.byType(Image).evaluate()) {
    final ImageProvider provider = (element.widget as Image).image;
    await tester.runAsync(() => precacheImage(provider, element));
  }
  await settle(tester);
}

/// The feeder's own screen at `tab`, with its name and tabs.
FeederShell _shell(FeederController feeder, int tab) => FeederShell(feeder: feeder, initialTab: tab);

/// A golden on Linux, with the app's clock stopped at the fixture's moment.
void _golden(String name, Future<void> Function(WidgetTester tester) body) =>
    testWidgets(name, skip: _skip, (tester) => atFixtureNow(() => body(tester)));

Future<void> _matches(String name, Brightness brightness) =>
    expectLater(find.byType(MaterialApp), matchesGoldenFile('files/$name.${brightness.name}.png'));

/// Four meals with a meal call each, one in every standing, so the rows show
/// with both a short time ("7:00 AM") and a long one ("12:00 PM").
Card _fourMealsCard() => fakeCard(
  nextMeal: NextMealView(slot: 2, atMs: DateTime(2026, 11, 2, 17).millisecondsSinceEpoch, minute: 17 * 60, portions: 2),
  lastFed: fixtureFed,
  meals: [
    MealView(
      slot: 0,
      minute: 7 * 60,
      portions: 2,
      call: 1,
      standing: MealStanding.fed(result: fixtureFed),
    ),
    const MealView(slot: 1, minute: 12 * 60, portions: 1, call: 1, standing: MealStanding.passed()),
    MealView(
      slot: 2,
      minute: 17 * 60,
      portions: 2,
      call: 1,
      standing: MealStanding.next(atMs: DateTime(2026, 11, 2, 17).millisecondsSinceEpoch),
    ),
    const MealView(slot: 3, minute: 22 * 60, portions: 1, call: 1, standing: MealStanding.later()),
  ],
  plans: fullCard().plans,
);

/// Every fact on Today that needs the owner at once: low food, batteries at
/// 15% with the adapter out, a weak signal, the desiccant overdue, and this
/// morning's meal cut short by a power loss.
Card _problemCard() {
  final FeedResult cut = fakeFed(
    feed: 41,
    confirmed: 0,
    atMost: 2,
    outcome: FeedOutcome.uncertain,
    reason: FeedReason.power,
    atMs: fixtureFed.atMs,
    by: const FedBy.plan(slot: 0),
  );
  final Card full = fullCard();
  return fakeCard(
    food: FoodLevel.low,
    adapter: false,
    batteryPercent: 15,
    rssi: -80,
    desiccant: const Desiccant.remaining(days: -2, due: true),
    fault: FaultView(feed: 41, outcome: FeedOutcome.uncertain, reason: FeedReason.power, result: cut),
    nextMeal: full.nextMeal,
    lastFed: cut,
    meals: [
      MealView(
        slot: 0,
        minute: 8 * 60,
        portions: 2,
        call: 0,
        standing: MealStanding.fed(result: cut),
      ),
    ],
    time: full.time,
    plans: full.plans,
    settings: full.settings,
  );
}

PairingView _pairing(PairingState state) => PairingView(
  flow: PairingFlow(FakeBook(), FakePairingPorts())..show(state),
  ble: true,
  onRetry: () {},
  onNewCode: () {},
);

void main() {
  setUpAll(() async {
    final previous = goldenFileComparator as LocalFileComparator;
    goldenFileComparator = LinuxGoldenComparator(previous.basedir.resolve('golden_test.dart'));
    await _loadFonts();
  });

  for (final Brightness brightness in Brightness.values) {
    group(brightness.name, () {
      _golden('the shared widgets', (tester) async {
        await _phone(
          tester,
          Scaffold(
            body: Builder(
              builder: (context) => ListView(
                padding: const EdgeInsets.all(Space.s4),
                children: [
                  const Headline('A headline'),
                  const SizedBox(height: Space.s3),
                  const Bar(value: 0.4),
                  const SizedBox(height: Space.s3),
                  Row(
                    children: [
                      StatusChip('Ready', color: context.colors.ok),
                      const SizedBox(width: Space.s3),
                      const FoodDot(present: true),
                      const SizedBox(width: Space.s3),
                      const FoodDot(present: false),
                    ],
                  ),
                  const SizedBox(height: Space.s3),
                  const Notice(tone: Tone.neutral, glyph: Glyph.info, text: 'A neutral note.'),
                  const SizedBox(height: Space.s2),
                  const Notice(
                    tone: Tone.warning,
                    glyph: Glyph.warning,
                    lead: 'A warning.',
                    text: 'It says what to do.',
                  ),
                  const SizedBox(height: Space.s2),
                  const Notice(tone: Tone.fault, lead: 'A fault.', text: 'It says what went wrong.'),
                  const SizedBox(height: Space.s3),
                  const NumberedSteps(['Hold **Lock** for 5 seconds.', 'Let go.']),
                  const SizedBox(height: Space.s3),
                  const SectionLabel('A section'),
                  const SizedBox(height: Space.s2),
                  const FactList([('A fact', Text('Its value'))]),
                  const SizedBox(height: Space.s3),
                  const Searching('Looking for feeders…'),
                ],
              ),
            ),
          ),
          brightness,
        );
        await _matches('shared_widgets', brightness);
      });

      _golden('the welcome', (tester) async {
        await _phone(tester, const WelcomeScreen(), brightness);
        await _matches('welcome', brightness);
      });

      _golden('adding a feeder', (tester) async {
        await _phone(tester, const AddFeederScreen(), brightness);
        await _matches('add_feeder', brightness);
      });

      _golden('the welcome on a small phone', (tester) async {
        await _phone(tester, const WelcomeScreen(), brightness, size: _smallPhone);
        await _matches('welcome_small', brightness);
      });

      _golden('a feeder card', (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(
          tester,
          Scaffold(
            body: ListView(
              padding: const EdgeInsets.all(Space.s4),
              children: [FeederCard(controller: feeder)],
            ),
          ),
          brightness,
        );
        await _decodeImages(tester);
        await _matches('feeder_card', brightness);
      });

      _golden("home with today's meals", (tester) async {
        final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
        answerAsFull(session);
        session.status(_fourMealsCard());
        await tester.pump();
        await _phone(tester, HomeScreen(fleet: fleet, earlierInstall: () async => PendingInstall.nothing), brightness);
        await _decodeImages(tester);
        await _matches('home', brightness);
      });

      _golden('today', (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, _shell(feeder, todayTab), brightness);
        await _matches('today', brightness);
      });

      _golden('today with problems', (tester) async {
        final FeederController feeder = await _feeder(tester, card: _problemCard());
        await _phone(tester, _shell(feeder, todayTab), brightness);
        await _matches('today_problem', brightness);
      });

      _golden('today in large text', (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, _shell(feeder, todayTab), brightness, text: _largeText);
        await _matches('today_large_text', brightness);
      });

      _golden('today on a small phone', (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, _shell(feeder, todayTab), brightness, size: _smallPhone);
        await _matches('today_small', brightness);
      });

      _golden('the schedule', (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, _shell(feeder, scheduleTab), brightness);
        await _matches('schedule', brightness);
      });

      _golden('feeder settings', (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, _shell(feeder, settingsTab), brightness);
        await _matches('feeder_settings', brightness);
      });

      _golden('feeder settings in large text', (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, _shell(feeder, settingsTab), brightness, text: _largeText);
        await _matches('feeder_settings_large_text', brightness);
      });

      _golden('the meal call', (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, MealCallScreen(feeder: feeder, pick: () async => null), brightness);
        await _matches('meal_call', brightness);
      });

      _golden('the meal call while recording', (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _phone(tester, MealCallScreen(feeder: feeder, pick: () async => null), brightness);
        await tester.tap(find.byTooltip('Record on this phone'));
        await tester.pump();
        await tester.pump(const Duration(seconds: 3));
        await _matches('meal_call_recording', brightness);
        await tester.tap(find.text('Cancel'));
        await settle(tester);
      });

      _golden('the feed sheet', (tester) async {
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

      for (final (String name, Size size) in [('setup', _phoneSize), ('setup_small', _smallPhone)]) {
        _golden(name, (tester) async {
          await _phone(
            tester,
            SetupScreen(newFlow: () => SetupFlow(FakeBook(), FakePairingPorts(), FakePhonePorts())),
            brightness,
            size: size,
          );
          await _matches(name, brightness);
        });
      }

      _golden('a failed pairing', (tester) async {
        await _phone(
          tester,
          _pairing(const PairingFailed(failure: PairingFailure.timeout, feederName: 'Kitchen')),
          brightness,
        );
        await _matches('pairing_failed', brightness);
      });

      _golden('app settings', (tester) async {
        final book = FakeBook()..label = 'My phone';
        await _phone(
          tester,
          AppSettingsScreen(book: book, newBackups: () => BackupsFlow(book, FakeBackupPorts())),
          brightness,
        );
        await _matches('app_settings', brightness);
      });

      for (final (String name, double text) in [('install', 1.0), ('install_large_text', _largeText)]) {
        _golden(name, (tester) async {
          _frame(tester, text: text);
          await tester.pumpWidget(installApp(FakeBook(), FakeInstallPorts(), brightness: brightness));
          await settle(tester);
          await _matches(name, brightness);
        });
      }

      _golden('the restore', (tester) async {
        await restoreOpen(tester, FakeRestorePorts(), brightness: brightness);
        _frame(tester);
        await settle(tester);
        await _matches('restore', brightness);
      });
    });
  }
}
