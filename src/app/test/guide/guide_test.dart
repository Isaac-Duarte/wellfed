import 'package:clock/clock.dart';
import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/meal_editor.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/screens/home/welcome.dart';
import 'package:wellfed/screens/install/install_progress.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/install/install.dart';

import '../golden/golden_fixtures.dart';
import '../golden/golden_support.dart';
import '../support/fake_install.dart';
import '../support/fakes.dart';
import '../support/harness.dart';

// The owner guide's screenshots, drawn from the same fixtures as the goldens
// and compared like them, so a screen that changes fails here until the
// guide's picture is redrawn: `make guide-screenshots` writes them back.
// The data is made up, so nothing private needs painting over.

/// A phone's screen without its status and navigation bars, at the guide's
/// width of 540 pixels: 412 by 824 logical pixels, at 135 to each 103.
const _guideScreen = Size(412, 824);
const double _guideRatio = 540 / 412;

/// Where the guide keeps its pictures, from this file's folder.
const _guide = '../../../../docs/guide';

/// One guide screenshot: `screen` shown dark, as the guide's pages are.
void screenshot(String name, Future<Widget> Function(WidgetTester tester) screen) => golden(name, (tester) async {
  final Widget shown = await screen(tester);
  await showPhone(tester, shown, Brightness.dark, size: _guideScreen, ratio: _guideRatio);
  await decodeImages(tester);
  await expectLater(find.byType(MaterialApp), matchesGoldenFile('$_guide/$name.png'));
});

InstallFlow _install() => InstallFlow(FakeBook(), FakeInstallPorts());

void main() {
  setUpGoldens('guide_test.dart');

  screenshot('app-welcome', (_) async => const WelcomeScreen());

  screenshot(
    'app-confirm-feeder',
    (_) async => overBlank(
      InstallConfirmPage(
        flow: _install(),
        state: const InstallConfirm(InstallFacts(), device: 'AF103-EXAMPLE', deviceAddress: '192.168.1.40'),
        onLeave: () async {},
      ),
    ),
  );

  screenshot(
    'app-installing',
    (_) async => overBlank(
      InstallSendingPage(
        flow: _install(),
        state: const InstallSending(InstallFacts(), holdRefused: false, percent: 60, restarting: false),
      ),
    ),
  );

  screenshot(
    'app-claim',
    (_) async => overBlank(
      InstallClaimPage(
        flow: _install(),
        state: InstallClaim(
          const InstallFacts(),
          code: const [4, 1, 2, 3, 3, 2],
          deadline: clock.now().add(const Duration(seconds: 48)),
          pressDispense: false,
          searchingBluetooth: true,
          settingUpAgain: false,
          setupHint: false,
          windowAsked: false,
        ),
      ),
    ),
  );

  screenshot(
    'app-permanent',
    (_) async => overBlank(
      InstallFinishingPage(
        flow: _install(),
        state: const InstallFinishing(
          InstallFacts(),
          joining: CheckState.waiting,
          copying: CheckState.done,
          copied: 1,
          checked: CheckState.done,
          promoting: CheckState.running,
          written: 0.4,
          confirming: CheckState.waiting,
        ),
      ),
    ),
  );

  screenshot('app-today', (tester) async {
    final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
    answerAsFull(session);
    session
      ..status(fourMealsCard())
      ..history(fourMealsHistory());
    await tester.pump();
    return shell(feeder, todayTab);
  });

  screenshot('app-schedule', (tester) async {
    final FeederController feeder = await goldenFeeder(tester, card: fourMealsCard());
    return shell(feeder, scheduleTab);
  });

  screenshot('app-meal-editor', (tester) async {
    final FeederController feeder = await goldenFeeder(tester);
    return overBlank(MealEditor(feeder: feeder, held: twoPlans, slot: 0));
  });

  screenshot('app-history', (tester) async {
    final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
    answerAsFull(session);
    session
      ..status(fullCard())
      ..history(twoDaysHistory());
    await tester.pump();
    return shell(feeder, historyTab);
  });
}
