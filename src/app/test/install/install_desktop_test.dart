import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/feeder_network.dart' show JoinRefused;
import 'package:wellfed/screens/install/firewall.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/state/install/install.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';
import 'install_support.dart';

/// `body` with the platform set to `platform`, for the test alone. A
/// `tester` draws what is on screen again on that platform first.
Future<void> on(TargetPlatform platform, Future<void> Function() body, {WidgetTester? tester}) async {
  debugDefaultTargetPlatformOverride = platform;
  try {
    if (tester != null) {
      // Reassembling waits for a frame, which only a pump draws.
      final Future<void> redrawn = tester.binding.reassembleApplication();
      await tester.pump();
      await redrawn;
      await settle(tester);
    }
    await body();
  } finally {
    debugDefaultTargetPlatformOverride = null;
  }
}

/// A computer's ports: the owner joins and leaves the feeder's network by hand.
FakeInstallPorts _computer({String ip = '10.0.0.5'}) => FakeInstallPorts(ip: ip)..feederJoin = FeederJoin.byHand;

void main() {
  group('A computer joins the feeder network by hand', () {
    testWidgets('it watches for the switch at once, and the frame goes from the joined address', (tester) async {
      final FakeInstallPorts ports = _computer();
      final FakeRun run = await installToRun(tester, ports);
      // The Wi-Fi the computer is on, as the system names it.
      ports.network = 'Maple St';
      await on(TargetPlatform.linux, tester: tester, () async {
        run.ask(2, const InstallAsk.joinFeederNetwork());
        await settle(tester);
        expect(ports.joined, 1);
        expect(ports.joinedFrom.single, '10.0.0.5');
        expect(find.text("Waiting for this computer to join the feeder's network…"), findsOneWidget);
        expect(find.textContaining('PL-AF103-', findRichText: true), findsWidgets);
        expect(find.textContaining('Maple St', findRichText: true), findsOneWidget);
        expect(find.textContaining('Quickest with a cable.', findRichText: true), findsOneWidget);
        expect(run.answers[2], isNull);

        ports.joining?.complete('192.168.1.100');
        await settle(tester);
        expect(run.answers[2], const InstallAnswer.joined(from: '192.168.1.100'));
      });
    });

    testWidgets('the owner can stop waiting and look again', (tester) async {
      final FakeInstallPorts ports = _computer();
      final FakeRun run = await installToRun(tester, ports);
      // The Wi-Fi the computer is on, as the system names it.
      ports.network = 'Maple St';
      await on(TargetPlatform.linux, tester: tester, () async {
        run.ask(2, const InstallAsk.joinFeederNetwork());
        await settle(tester);

        await installTap(tester, 'Stop waiting');
        expect(ports.joinStops, 1);
        expect(find.textContaining("This computer didn't join the feeder's network.", findRichText: true), findsOne);
        expect(run.answers[2], isNull);

        await installTap(tester, 'Look for it again');
        expect(ports.joined, 2);
        ports.joining?.complete('192.168.1.100');
        await settle(tester);
        expect(run.answers[2], const InstallAnswer.joined(from: '192.168.1.100'));
      });
    });

    testWidgets('after the frame it asks the owner to switch back, and waits', (tester) async {
      final FakeInstallPorts ports = _computer();
      final FakeRun run = await installToRun(tester, ports);
      // The Wi-Fi the computer is on, as the system names it.
      ports.network = 'Maple St';
      await on(TargetPlatform.linux, tester: tester, () async {
        run.ask(2, const InstallAsk.joinFeederNetwork());
        await settle(tester);
        ports.joining?.complete('192.168.1.100');
        await settle(tester);

        final Completer<String?> back = ports.leaving = Completer<String?>();
        run.ask(3, const InstallAsk.leaveFeederNetwork());
        await settle(tester);
        expect(find.textContaining('Switch back to Maple St now.', findRichText: true), findsOneWidget);
        expect(find.text('Waiting for this computer to be back on your network…'), findsOneWidget);
        expect(run.answers[3], isNull);

        back.complete('10.0.0.5');
        await settle(tester);
        expect(run.answers[3], const InstallAnswer.done());
        expect(find.text('Waiting for the feeder to join your Wi-Fi…'), findsOneWidget);
      });
    });

    testWidgets('coming back at another address fails the ask, as on a phone', (tester) async {
      final FakeInstallPorts ports = _computer()..ipAfterLeaving = '10.0.0.9';
      final FakeRun run = await installToRun(tester, ports);
      // The Wi-Fi the computer is on, as the system names it.
      ports.network = 'Maple St';
      await on(TargetPlatform.linux, tester: tester, () async {
        run.ask(3, const InstallAsk.leaveFeederNetwork());
        await settle(tester);
        expect(run.answers[3], isA<InstallAnswer_Failed>());
      });
    });

    testWidgets("a home network on the feeder's addresses is told to disconnect", (tester) async {
      final FakeInstallPorts ports = _computer(ip: '192.168.1.23');
      final FakeRun run = await installToRun(tester, ports);
      // The Wi-Fi the computer is on, as the system names it.
      ports.network = 'Maple St';
      await on(TargetPlatform.linux, tester: tester, () async {
        run.ask(2, const InstallAsk.joinFeederNetwork());
        await settle(tester);
        expect(
          find.textContaining("Your home network uses the feeder's addresses.", findRichText: true),
          findsOneWidget,
        );
        expect(find.textContaining('Quickest with a cable.', findRichText: true), findsNothing);
      });
    });

    testWidgets('a run that ends while it waits stops waiting', (tester) async {
      final FakeInstallPorts ports = _computer();
      final FakeRun run = await installToRun(tester, ports);
      // The Wi-Fi the computer is on, as the system names it.
      ports.network = 'Maple St';
      await on(TargetPlatform.linux, tester: tester, () async {
        run.ask(2, const InstallAsk.joinFeederNetwork());
        await settle(tester);

        run.stopped(const StopCase.cancelled(), resume: Resume.fromStart);
        await settle(tester);
        expect(ports.joinStops, 1);
        expect(run.answers[2], isNull);
      });
    });

    testWidgets('a phone still joins through the picker, with nothing to wait on', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToRun(tester, ports);
      run.ask(2, const InstallAsk.joinFeederNetwork());
      await settle(tester);
      expect(ports.joined, 0);
      await installTap(tester, "Join the feeder's network");
      expect(run.answers[2], const InstallAnswer.done());
      run.ask(3, const InstallAsk.leaveFeederNetwork());
      await settle(tester);
      expect(find.textContaining('Switch back'), findsNothing);
    });
  });

  group('A computer refuses what would send the password elsewhere', () {
    for (final (FeederJoinProblem problem, String lead, String fix) in [
      (
        FeederJoinProblem.otherNetwork,
        'This computer is already on another 192.168.1.x network.',
        'Disconnect it, then look again.',
      ),
      (FeederJoinProblem.vpn, "The feeder can't reach this computer's address.", 'Turn off your VPN'),
      (FeederJoinProblem.notSeen, "This computer didn't join the feeder's network.", 'If a VPN is on, turn it off'),
      (FeederJoinProblem.nameUnreadable, "This computer didn't say which Wi-Fi it joined.", 'so nothing was sent'),
      (
        FeederJoinProblem.alreadyOnFeeder,
        "This computer was already on the feeder's network.",
        'close this install and start again',
      ),
    ]) {
      testWidgets('${problem.name} says what to change, and nothing is answered', (tester) async {
        final FakeInstallPorts ports = _computer()..joinFails = JoinRefused(problem);
        final FakeRun run = await installToRun(tester, ports);
        await on(TargetPlatform.linux, tester: tester, () async {
          run.ask(2, const InstallAsk.joinFeederNetwork());
          await settle(tester);
          expect(find.textContaining(lead, findRichText: true), findsOneWidget);
          expect(find.textContaining(fix, findRichText: true), findsOneWidget);
          expect(find.text('Look for it again'), findsOneWidget);
          expect(run.answers[2], isNull);
        });
      });
    }

    for (final (TargetPlatform platform, String where) in [
      (TargetPlatform.macOS, 'Privacy & Security › Location Services'),
      (TargetPlatform.windows, 'Settings › Privacy & security › Location'),
      (TargetPlatform.linux, 'NetworkManager (nmcli) or iw'),
    ]) {
      testWidgets('an unread name on ${platform.name} says how to let Wellfed read it', (tester) async {
        final FakeInstallPorts ports = _computer()..joinFails = const JoinRefused(FeederJoinProblem.nameUnreadable);
        final FakeRun run = await installToRun(tester, ports);
        await on(platform, tester: tester, () async {
          run.ask(2, const InstallAsk.joinFeederNetwork());
          await settle(tester);
          expect(find.textContaining(where, findRichText: true), findsOneWidget);
        });
      });
    }

    testWidgets('a home network name that cannot be read still lets the watch begin', (tester) async {
      final FakeInstallPorts ports = _computer();
      final FakeRun run = await installToRun(tester, ports);
      ports.networkFails = StateError('no NetworkManager');
      await on(TargetPlatform.linux, tester: tester, () async {
        run.ask(2, const InstallAsk.joinFeederNetwork());
        await settle(tester);
        expect(ports.joined, 1);
        expect(find.textContaining('switch straight back to your home network', findRichText: true), findsOneWidget);
        await installTap(tester, 'Stop waiting');
        expect(find.text('Look for it again'), findsOneWidget);
      });
    });

    testWidgets('a stop before the watch begins is not lost', (tester) async {
      final FakeInstallPorts ports = _computer();
      final FakeRun run = await installToRun(tester, ports);
      final Completer<void> gate = ports.networkGate = Completer<void>();
      await on(TargetPlatform.linux, tester: tester, () async {
        run.ask(2, const InstallAsk.joinFeederNetwork());
        await settle(tester);
        await installTap(tester, 'Stop waiting');

        gate.complete();
        await settle(tester);
        expect(ports.joined, 0, reason: 'the watch never starts once stopped');
        expect(find.textContaining("This computer didn't join", findRichText: true), findsOneWidget);
        expect(run.answers[2], isNull);
      });
    });
  });

  testWidgets('a hold that gives way mid-install says to keep the computer awake', (tester) async {
    final FakeInstallPorts ports = _computer();
    final FakeRun run = await installAtConfirm(tester, ports);
    await on(TargetPlatform.linux, tester: tester, () async {
      await installTap(tester, 'Yes, install Wellfed');
      run.event(const InstallEvent.requesting(build: 'ab'));
      await settle(tester);
      expect(find.textContaining("wouldn't promise to stay awake", findRichText: true), findsNothing);

      ports.lapses.add(null);
      await settle(tester);
      expect(find.textContaining("wouldn't promise to stay awake", findRichText: true), findsOneWidget);
    });
  });

  group("A feeder that never arrives names the computer's firewall", () {
    for (final (TargetPlatform platform, String words) in [
      (TargetPlatform.macOS, 'System Settings › Network › Firewall › Options'),
      (TargetPlatform.windows, 'Allow an app through firewall'),
      (TargetPlatform.linux, 'sudo ufw allow 1883,8088/tcp'),
    ]) {
      test('on ${platform.name}', () => on(platform, () async => expect(firewallWords(), contains(words))));
    }

    test('and nothing on a phone', () => on(TargetPlatform.android, () async => expect(firewallWords(), isNull)));

    for (final StopCase stop in [
      const StopCase.frameNotTaken(),
      const StopCase.noFeederCame(),
      const StopCase.stockRefused(code: 2003),
    ]) {
      testWidgets('when it stops with $stop', (tester) async {
        await installStopped(tester, _computer(), stop, resume: Resume.fromStart);
        await on(TargetPlatform.macOS, tester: tester, () async {
          expect(
            find.textContaining("This computer's firewall may be in the way.", findRichText: true),
            findsOneWidget,
          );
          expect(find.textContaining('TCP ports 1883 and 8088', findRichText: true), findsOneWidget);
        });
      });
    }

    testWidgets('but not for a stop it could not cause', (tester) async {
      await installStopped(tester, _computer(), const StopCase.twoFeeders(), resume: Resume.fromStart);
      await on(TargetPlatform.macOS, tester: tester, () async {
        expect(find.textContaining("This computer's firewall may be in the way.", findRichText: true), findsNothing);
      });
    });

    testWidgets('nor on a phone', (tester) async {
      await installStopped(tester, FakeInstallPorts(), const StopCase.frameNotTaken(), resume: Resume.fromStart);
      expect(find.textContaining("This computer's firewall may be in the way.", findRichText: true), findsNothing);
    });
  });

  testWidgets("a computer that won't promise to stay awake says to keep it awake", (tester) async {
    final FakeInstallPorts ports = _computer()..holds = false;
    final FakeRun run = await installAtConfirm(tester, ports);
    await on(TargetPlatform.linux, tester: tester, () async {
      await installTap(tester, 'Yes, install Wellfed');
      run.event(const InstallEvent.requesting(build: 'ab'));
      await settle(tester);
      expect(find.textContaining("This computer wouldn't promise to stay awake.", findRichText: true), findsOneWidget);
      expect(find.textContaining('with its lid open', findRichText: true), findsWidgets);
    });
  });
}
