import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/feed_sheet.dart';
import 'package:wellfed/screens/feeder/meal_call.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/screens/feeder/wifi.dart';
import 'package:wellfed/screens/home/add_feeder.dart';
import 'package:wellfed/screens/home/feeder_card.dart';
import 'package:wellfed/screens/home/home.dart';
import 'package:wellfed/screens/home/welcome.dart';
import 'package:wellfed/screens/install/install_screen.dart';
import 'package:wellfed/screens/restore/restore_screen.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/screens/setup/pairing_view.dart';
import 'package:wellfed/screens/setup/setup_screen.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/state/pairing/pairing.dart';
import 'package:wellfed/state/restore/restore.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/state/setup/setup.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

import '../feeder/feeder_support.dart';
import '../restore/restore_support.dart';
import '../support/fake_install.dart';
import '../support/fakes.dart';
import '../support/harness.dart';
import 'golden_support.dart';

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

/// A day with a skip in each standing: breakfast skipped with its time come,
/// supper offered for a skip, and a late meal skipped with its undo still open.
Card _skippingCard() {
  final int supper = DateTime(2026, 11, 2, 17).millisecondsSinceEpoch;
  final int late = DateTime(2026, 11, 2, 22).millisecondsSinceEpoch;
  return fakeCard(
    nextMeal: NextMealView(slot: 2, atMs: supper, minute: 17 * 60, portions: 2),
    meals: [
      const MealView(slot: 0, minute: 7 * 60, portions: 2, call: 1, standing: MealStanding.skipKept()),
      MealView(
        slot: 2,
        minute: 17 * 60,
        portions: 2,
        call: 1,
        standing: MealStanding.next(atMs: supper),
        action: MealAction.skip(dueMs: supper),
      ),
      MealView(
        slot: 3,
        minute: 22 * 60,
        portions: 1,
        call: 0,
        standing: MealStanding.skipped(atMs: late),
        action: MealAction.undo(dueMs: late),
      ),
    ],
    plans: fullCard().plans,
  );
}

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
  setUpGoldens('golden_test.dart');

  for (final Brightness brightness in Brightness.values) {
    group(brightness.name, () {
      golden('the shared widgets', (tester) async {
        await showPhone(
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
        await matchesGolden('shared_widgets', brightness);
      });

      golden('the welcome', (tester) async {
        await showPhone(tester, const WelcomeScreen(), brightness);
        await decodeImages(tester);
        await matchesGolden('welcome', brightness);
      });

      golden('adding a feeder', (tester) async {
        await showPhone(tester, overBlank(const AddFeederScreen()), brightness);
        await matchesGolden('add_feeder', brightness);
      });

      golden('the welcome on a small phone', (tester) async {
        await showPhone(tester, const WelcomeScreen(), brightness, size: smallPhone);
        await decodeImages(tester);
        await matchesGolden('welcome_small', brightness);
      });

      golden('a feeder card', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(
          tester,
          Scaffold(
            body: ListView(
              padding: const EdgeInsets.all(Space.s4),
              children: [FeederCard(controller: feeder)],
            ),
          ),
          brightness,
        );
        await decodeImages(tester);
        await matchesGolden('feeder_card', brightness);
      });

      golden("home with today's meals", (tester) async {
        final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
        answerAsFull(session);
        session.status(_fourMealsCard());
        await tester.pump();
        await showPhone(
          tester,
          HomeScreen(fleet: fleet, earlierInstall: () async => PendingInstall.nothing),
          brightness,
        );
        await decodeImages(tester);
        await matchesGolden('home', brightness);
      });

      golden('today', (tester) async {
        // A whole day: a meal fed, one skipped, one next and one later, and a
        // press of the Dispense button between them.
        final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
        answerAsFull(session);
        session
          ..status(_fourMealsCard())
          ..history([
            HistoryEntry(
              seq: BigInt.one,
              atMs: DateTime(2026, 11, 2, 9, 12).millisecondsSinceEpoch,
              body: HistoryBody.fed(
                result: fakeFed(atMs: DateTime(2026, 11, 2, 9, 12).millisecondsSinceEpoch, requested: 1, confirmed: 1),
              ),
            ),
            HistoryEntry(
              seq: BigInt.two,
              atMs: DateTime(2026, 11, 2, 12, 1).millisecondsSinceEpoch,
              body: HistoryBody.missed(
                slot: 1,
                dueMs: DateTime(2026, 11, 2, 12).millisecondsSinceEpoch,
                reason: const MissedReason.late_(),
              ),
            ),
          ]);
        await tester.pump();
        await showPhone(tester, shell(feeder, todayTab), brightness);
        await decodeImages(tester);
        await matchesGolden('today', brightness);
      });

      golden('today with problems', (tester) async {
        final FeederController feeder = await goldenFeeder(tester, card: _problemCard());
        await showPhone(tester, shell(feeder, todayTab), brightness);
        await matchesGolden('today_problem', brightness);
      });

      golden('today in large text', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(tester, shell(feeder, todayTab), brightness, text: largeText);
        await matchesGolden('today_large_text', brightness);
      });

      golden('today on a small phone', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(tester, shell(feeder, todayTab), brightness, size: smallPhone);
        await matchesGolden('today_small', brightness);
      });

      golden('today with skipped meals', (tester) async {
        final FeederController feeder = await goldenFeeder(tester, card: _skippingCard());
        await showPhone(tester, shell(feeder, todayTab), brightness);
        await matchesGolden('today_skipped', brightness);
      });

      golden("a meal skip's confirmation", (tester) async {
        final FeederController feeder = await goldenFeeder(tester, card: _skippingCard());
        await showPhone(tester, shell(feeder, todayTab), brightness);
        await tester.tap(find.text('Skip'));
        await tester.pumpAndSettle();
        await matchesGolden('skip_meal_confirm', brightness);
      });

      golden('the schedule', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(tester, shell(feeder, scheduleTab), brightness);
        await matchesGolden('schedule', brightness);
      });

      golden('feeder settings', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(tester, shell(feeder, settingsTab), brightness);
        await matchesGolden('feeder_settings', brightness);
      });

      golden('feeder settings in large text', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(tester, shell(feeder, settingsTab), brightness, text: largeText);
        await matchesGolden('feeder_settings_large_text', brightness);
      });

      golden('the meal call', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(tester, MealCallScreen(feeder: feeder, pick: () async => null), brightness);
        await matchesGolden('meal_call', brightness);
      });

      golden('the meal call while recording', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(tester, MealCallScreen(feeder: feeder, pick: () async => null), brightness);
        await tester.tap(find.byTooltip('Record on this phone'));
        await tester.pump();
        await tester.pump(const Duration(seconds: 3));
        await matchesGolden('meal_call_recording', brightness);
        await tester.tap(find.text('Cancel'));
        await settle(tester);
      });

      golden('Wi-Fi settings', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(tester, WifiScreen(feeder: feeder), brightness);
        await matchesGolden('wifi_settings', brightness);
      });

      golden('the network picker', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(tester, WifiScreen(feeder: feeder), brightness);
        await tester.tap(find.text('Home'));
        await settle(tester);
        await matchesGolden('wifi_picker', brightness);
      });

      golden('the feed sheet', (tester) async {
        final FeederController feeder = await goldenFeeder(tester);
        await showPhone(
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
        await matchesGolden('feed_sheet', brightness);
      });

      for (final (String name, Size size) in [('setup', phoneSize), ('setup_small', smallPhone)]) {
        golden(name, (tester) async {
          await showPhone(
            tester,
            SetupScreen(newFlow: () => SetupFlow(FakeBook(), FakePairingPorts(), FakePhonePorts())),
            brightness,
            size: size,
          );
          await decodeImages(tester);
          await matchesGolden(name, brightness);
        });
      }

      golden('a failed pairing', (tester) async {
        await showPhone(
          tester,
          _pairing(const PairingFailed(failure: PairingFailure.timeout, feederName: 'Kitchen')),
          brightness,
        );
        await matchesGolden('pairing_failed', brightness);
      });

      golden('app settings', (tester) async {
        final book = FakeBook()..label = 'My phone';
        await showPhone(
          tester,
          AppSettingsScreen(book: book, newBackups: () => BackupsFlow(book, FakeBackupPorts())),
          brightness,
        );
        await matchesGolden('app_settings', brightness);
      });

      for (final (String name, double text) in [('install', 1.0), ('install_large_text', largeText)]) {
        golden(name, (tester) async {
          await showPhone(
            tester,
            overBlank(InstallScreen(newFlow: () => InstallFlow(FakeBook(), FakeInstallPorts()))),
            brightness,
            text: text,
          );
          await matchesGolden(name, brightness);
        });
      }

      golden('the restore', (tester) async {
        final RestoreFlow flow = restoreFlow(FakeRestorePorts());
        await flow.load();
        await showPhone(tester, overBlank(RestoreView(flow: flow)), brightness);
        await matchesGolden('restore', brightness);
      });
    });
  }
}
