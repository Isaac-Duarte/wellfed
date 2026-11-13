import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/join_ports.dart';
import 'package:wellfed/screens/feeder/clock.dart';
import 'package:wellfed/screens/feeder/diagnostics.dart';
import 'package:wellfed/screens/feeder/feed_sheet.dart';
import 'package:wellfed/screens/feeder/history.dart';
import 'package:wellfed/screens/feeder/invite.dart';
import 'package:wellfed/screens/feeder/lights_help.dart';
import 'package:wellfed/screens/feeder/live_log.dart';
import 'package:wellfed/screens/feeder/meal_call.dart';
import 'package:wellfed/screens/feeder/meal_editor.dart';
import 'package:wellfed/screens/feeder/motor_trace.dart';
import 'package:wellfed/screens/feeder/people.dart';
import 'package:wellfed/screens/feeder/portion.dart';
import 'package:wellfed/screens/feeder/schedule.dart';
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/feeder/today.dart';
import 'package:wellfed/screens/feeder/update_end.dart';
import 'package:wellfed/screens/feeder/updates.dart';
import 'package:wellfed/screens/feeder/wifi.dart';
import 'package:wellfed/screens/home/add_feeder.dart';
import 'package:wellfed/screens/home/home.dart';
import 'package:wellfed/screens/home/welcome.dart';
import 'package:wellfed/screens/install/install_before.dart';
import 'package:wellfed/screens/install/install_handover.dart';
import 'package:wellfed/screens/install/install_progress.dart';
import 'package:wellfed/screens/install/install_stopped.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/screens/setup/join.dart';
import 'package:wellfed/screens/setup/pairing_view.dart';
import 'package:wellfed/screens/setup/recover_screen.dart';
import 'package:wellfed/screens/setup/setup_screen.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/join.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/restore.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/update.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/state/join/join.dart';
import 'package:wellfed/state/pairing/pairing.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/state/setup/setup.dart';
import 'package:wellfed/state/update/update_state.dart';

import '../feeder/feeder_support.dart';
import '../feeder/schedule_history_support.dart';
import '../install/install_support.dart';
import '../restore/restore_support.dart';
import '../support/accessibility.dart';
import '../support/fake_install.dart';
import '../support/fakes.dart';
import '../support/harness.dart';

/// `screen` for `feeder` on a phone, with its session answering.
Future<void> _show(WidgetTester tester, Widget screen, Brightness brightness) async {
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

class _NoJoin implements JoinPorts {
  const _NoJoin();

  @override
  String? inviteFeeder(String text) => null;

  @override
  Future<JoinAnswer> join(String invite, String label, String address) => throw UnimplementedError();

  @override
  Future<String?> locate(String identity, Duration wait) async => null;
}

InstallFlow _install() => InstallFlow(FakeBook(), FakeInstallPorts());

PairingFlow _pairing(PairingState state) => PairingFlow(FakeBook(), FakePairingPorts())..show(state);

void main() {
  testWidgets('a target too small, unlabeled and faint fails each check', (tester) async {
    final SemanticsHandle handle = tester.ensureSemantics();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          backgroundColor: const Color(0xFFFFFFFF),
          body: Center(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                SizedBox.square(dimension: 20, child: GestureDetector(onTap: () {})),
                TextButton(
                  onPressed: () {},
                  child: const Text('Faint', style: TextStyle(color: Color(0xFFEEEEEE))),
                ),
              ],
            ),
          ),
        ),
      ),
    );

    expect((await androidTapTargetGuideline.evaluate(tester)).passed, isFalse);
    expect((await labeledTapTargetGuideline.evaluate(tester)).passed, isFalse);
    expect((await textContrastGuideline.evaluate(tester)).passed, isFalse);
    handle.dispose();
  });

  for (final Brightness brightness in Brightness.values) {
    group('${brightness.name} theme', () {
      testWidgets('the welcome', (tester) async {
        await _show(tester, const WelcomeScreen(), brightness);
        await expectAccessible(tester);
      });

      testWidgets('adding a feeder', (tester) async {
        await _show(tester, const AddFeederScreen(), brightness);
        await expectAccessible(tester);
      });

      testWidgets('app settings', (tester) async {
        final book = FakeBook()..label = 'My phone';
        await _show(
          tester,
          AppSettingsScreen(book: book, newBackups: () => BackupsFlow(book, FakeBackupPorts())),
          brightness,
        );
        await expectAccessible(tester);
      });

      testWidgets('setup', (tester) async {
        await _show(
          tester,
          SetupScreen(newFlow: () => SetupFlow(FakeBook(), FakePairingPorts(), FakePhonePorts())),
          brightness,
        );
        await expectAccessible(tester);
      });

      testWidgets('joining', (tester) async {
        await _show(
          tester,
          JoinScreen(newFlow: () => JoinFlow(FakeBook(), const _NoJoin()), scanner: (_) => const SizedBox()),
          brightness,
        );
        await expectAccessible(tester);
      });

      testWidgets('lights help', (tester) async {
        await _show(tester, const LightsHelp(), brightness);
        await expectAccessible(tester);
      });

      for (final (String name, Widget Function(FeederController) screen)
          in <(String, Widget Function(FeederController))>[
            ('today', (feeder) => TodayTab(feeder: feeder)),
            ('schedule', (feeder) => ScheduleTab(feeder: feeder)),
            ('history', (feeder) => HistoryTab(feeder: feeder)),
            ('feeder settings', (feeder) => SettingsTab(feeder: feeder)),
            ('people', (feeder) => PeopleScreen(feeder: feeder)),
            ('Wi-Fi', (feeder) => WifiScreen(feeder: feeder)),
            ('updates', (feeder) => UpdatesScreen(feeder: feeder)),
            ('diagnostics', (feeder) => DiagnosticsScreen(feeder: feeder)),
            ('the clock', (feeder) => ClockScreen(feeder: feeder)),
            ('portion size', (feeder) => PortionScreen(feeder: feeder)),
            ('the live log', (feeder) => LiveLogScreen(feeder: feeder)),
            ('the motor traces', (feeder) => MotorTracesScreen(feeder: feeder)),
            ('a motor trace', (feeder) => MotorTraceScreen(feeder: feeder, feed: 1)),
            ('the meal call', (feeder) => MealCallScreen(feeder: feeder, pick: () async => null)),
          ]) {
        testWidgets(name, (tester) async {
          final FeederController feeder = await _feeder(tester);
          await _show(tester, Scaffold(body: screen(feeder)), brightness);
          await expectAccessible(tester);
        });
      }

      testWidgets('the meal call with a clip, and recording', (tester) async {
        final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
        answerAsFull(session);
        session.status(fullCard());
        await _show(
          tester,
          Scaffold(
            body: MealCallScreen(feeder: feeder, pick: () async => null),
          ),
          brightness,
        );
        await expectAccessible(tester);

        await tester.tap(find.byTooltip('Record on this phone'));
        await tester.pump(const Duration(seconds: 2));
        await expectAccessible(tester);
        await tester.tap(find.text('Cancel'));
        await settle(tester);
      });

      testWidgets('the network picker and its list', (tester) async {
        final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
        session.status(fullCard());
        await _show(tester, WifiScreen(feeder: feeder), brightness);
        await expectAccessible(tester);
        await tester.tap(find.text('Home'));
        await settle(tester);
        await expectAccessible(tester);
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
          brightness,
        );
        await tester.tap(find.text('Open'));
        await settle(tester);
        await expectAccessible(tester);
      });

      for (final (String name, Widget Function() page) in <(String, Widget Function())>[
        (
          'finding a feeder',
          () => PairingView(
            flow: _pairing(const PairingFinding(feederName: 'Kitchen')),
            ble: true,
            onRetry: () {},
            onNewCode: () {},
          ),
        ),
        (
          'the code',
          () => PairingView(
            flow: _pairing(
              PairingCode(
                code: const [1, 2, 3, 4, 1, 2],
                deadline: DateTime.now().add(const Duration(minutes: 1)),
                feederName: 'Kitchen',
              ),
            ),
            ble: true,
            onRetry: () {},
            onNewCode: () {},
          ),
        ),
        (
          'joining the Wi-Fi',
          () => PairingView(
            flow: _pairing(
              const PairingJoining(wifiSaved: WifiSaved.saved, network: 'Maple St', feederName: 'Kitchen'),
            ),
            ble: true,
            onRetry: () {},
            onNewCode: () {},
          ),
        ),
        (
          'paired',
          () => PairingView(
            flow: _pairing(const PairingDone(enrolled: Enrolled.already, feeder: kitchen, feederName: 'Kitchen')),
            ble: true,
            onRetry: () {},
            onNewCode: () {},
          ),
        ),
        (
          'a failed pairing',
          () => PairingView(
            flow: _pairing(const PairingFailed(failure: PairingFailure.timeout, feederName: 'Kitchen')),
            ble: true,
            onRetry: () {},
            onNewCode: () {},
          ),
        ),
        ('the Reset hold', () => InstallResetPage(flow: _install(), state: const InstallReset(InstallFacts()))),
        (
          'the handover',
          () => InstallHandoverPage(
            flow: _install(),
            state: const InstallHandover(
              InstallFacts(),
              handover: Handover.join,
              joinFailed: true,
              restartAgain: false,
              waitReason: null,
            ),
            onLeave: () async {},
          ),
        ),
        (
          'is this your feeder',
          () => InstallConfirmPage(
            flow: _install(),
            state: const InstallConfirm(InstallFacts(), device: 'PLAF103', deviceAddress: '10.0.0.5'),
            onLeave: () async {},
          ),
        ),
        ('which feeder', () => InstallWhichFeederPage(flow: _install())),
        (
          'sending',
          () => InstallSendingPage(
            flow: _install(),
            state: const InstallSending(InstallFacts(), holdRefused: true, percent: 40, restarting: false),
          ),
        ),
        (
          'the claim',
          () => InstallClaimPage(
            flow: _install(),
            state: InstallClaim(
              const InstallFacts(),
              code: const [1, 2, 3, 4, 1, 2],
              deadline: DateTime.now().add(const Duration(minutes: 1)),
              pressDispense: false,
              searchingBluetooth: false,
              settingUpAgain: false,
              setupHint: false,
              windowAsked: false,
            ),
          ),
        ),
        (
          'finishing',
          () => InstallFinishingPage(
            flow: _install(),
            state: const InstallFinishing(
              InstallFacts(),
              joining: CheckState.done,
              copying: CheckState.running,
              copied: 0.4,
              checked: CheckState.waiting,
              promoting: CheckState.waiting,
              written: 0,
              confirming: CheckState.waiting,
            ),
          ),
        ),
        (
          'done',
          () => InstallDonePage(
            flow: _install(),
            state: const InstallDone(InstallFacts(), backupPath: '/backups/aa'),
          ),
        ),
        (
          'stopped',
          () => InstallStoppedPage(
            flow: _install(),
            state: const InstallStopped(
              InstallFacts(),
              stop: InstallEnd_Stopped(case_: StopCase.noFeederCame(), resume: Resume.fromStart, detail: 'small print'),
              listening: Listening.idle,
              listened: Duration.zero,
              listeningForEarlier: false,
              stockSeen: false,
              setupSeen: false,
              listenerDetail: null,
            ),
          ),
        ),
      ]) {
        testWidgets(name, (tester) async {
          await _show(tester, Scaffold(body: page()), brightness);
          await expectAccessible(tester);
        });
      }

      testWidgets('recovery', (tester) async {
        final book = FakeBook()..label = 'My phone';
        await _show(
          tester,
          RecoverScreen(newFlow: () => RecoverFlow(book, FakePairingPorts(), FakePhonePorts())),
          brightness,
        );
        await expectAccessible(tester);
      });

      testWidgets('an invite', (tester) async {
        await _show(
          tester,
          InviteScreen(
            invite: const InviteView(text: 'wellfed-invite:1:x', seconds: 600, role: ClientRole.member),
            share: (_) async {},
          ),
          brightness,
        );
        await expectAccessible(tester);
        await tester.pumpWidget(const SizedBox());
      });

      testWidgets('home and the feeder shell', (tester) async {
        final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
        session.status(fullCard());
        await _show(tester, HomeScreen(fleet: fleet, earlierInstall: () async => PendingInstall.nothing), brightness);
        await expectAccessible(tester);

        await _show(tester, FeederShell(feeder: feederOnly(fleet)), brightness);
        await expectAccessible(tester);
      });

      testWidgets('the meal editor', (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _show(tester, MealEditor(feeder: feeder, held: planPlans([null])), brightness);
        await expectAccessible(tester);
      });

      testWidgets("an update's end", (tester) async {
        final FeederController feeder = await _feeder(tester);
        await _show(
          tester,
          UpdateEndScreen(feeder: feeder, ended: const UpdateEnded(UpdateEnd.uncertain())),
          brightness,
        );
        await expectAccessible(tester);
      });

      for (final (String name, SessionUpdate Function() update) in <(String, SessionUpdate Function())>[
        ('running', () => const SessionUpdate.feedStarted(feed: 1, portions: 2, dueSeconds: 60)),
        ('refused', () => const SessionUpdate.feedRefused(refusal: FeedRefusal.forbidden)),
        ('unanswered', () => const SessionUpdate.feedUnanswered()),
      ]) {
        testWidgets('a feed $name', (tester) async {
          final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
          session.status(fullCard());
          await tester.pump();
          feeder.feedNow(2);
          session.updates.add(update());
          await _show(
            tester,
            Builder(
              builder: (context) => Scaffold(
                body: TextButton(onPressed: () => showFeedSheet(context, feeder), child: const Text('Open')),
              ),
            ),
            brightness,
          );
          await tester.tap(find.text('Open'));
          await settle(tester);
          await expectAccessible(tester);
          feeder.dismissFeed();
          await tester.pump();
        });
      }

      for (final (String name, void Function(FakeRestoreRun) step) in <(String, void Function(FakeRestoreRun))>[
        ('running', (run) => run.event(const InstallEvent.promoting())),
        ('stopped', (run) => run.stopped(RestoreStop.noPress)),
        ('uncertain', (run) => run.end(const RestoreEnd.uncertain(why: UncertainWhy.stopped, detail: 'stopped'))),
      ]) {
        testWidgets('a restore $name', (tester) async {
          final ports = FakeRestorePorts();
          await restoreOpen(tester, ports, brightness: brightness);
          final FakeRestoreRun run = await restoreStarted(tester, ports);
          step(run);
          await settle(tester);
          await expectAccessible(tester);
          await tester.pumpWidget(const SizedBox());
        });
      }

      testWidgets('the app bar of the about screen', (tester) async {
        await _show(tester, AboutScreen(version: () => '0.1.0'), brightness);
        await expectAccessible(tester);
      });

      testWidgets('the install', (tester) async {
        await tester.pumpWidget(installApp(FakeBook(), FakeInstallPorts(), brightness: brightness));
        tester.view.physicalSize = const Size(412, 915);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        await settle(tester);
        await expectAccessible(tester);
      });

      testWidgets('the restore', (tester) async {
        await restoreOpen(tester, FakeRestorePorts(), brightness: brightness);
        await expectAccessible(tester);
      });
    });
  }
}
