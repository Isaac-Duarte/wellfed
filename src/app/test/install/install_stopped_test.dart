import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/install/install_before.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/widgets/code_tiles.dart';
import 'package:wellfed/widgets/layout.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';
import 'install_support.dart';

void main() {
  group('2.9 stopped', () {
    testWidgets('an uncertain install is listened for, counts to 20 minutes and sends nothing', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installStopped(tester, ports, const StopCase.uncertain());

      expect(find.text("We don't know yet if it installed"), findsOneWidget);
      final FakeListener listener = ports.listeners.single;
      expect(ports.releases, 0, reason: 'the broker stays up while it listens');

      listener.updates.add(const ListenUpdate.tick(seconds: 180, stock: false));
      await tester.pump();
      expect(find.text('Listening for the feeder · 3 of 20 min'), findsOneWidget);

      // The three other ways stay on the screen.
      expect(find.text('Different vendor firmware'), findsOneWidget);
      expect(find.text('Two feeders answered'), findsOneWidget);
      expect(find.text("The backup didn't match"), findsOneWidget);

      // Only one run: the request is never sent a second time.
      expect(ports.runs, hasLength(1));
      expect(run.answers, isEmpty);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('a setup advert keeps it listening, and the owner picks up from bank 0', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.uncertain());
      ports.heard.add(null);
      await settle(tester);

      // It could be any feeder in setup mode.
      expect(ports.listeners.single.stops, 0);
      expect(ports.runs, hasLength(1));
      expect(find.textContaining('A feeder nearby is waiting for setup.', findRichText: true), findsOneWidget);

      await tester.ensureVisible(find.text('Set up this feeder'));
      await settle(tester);
      await tester.tap(find.text('Set up this feeder'));
      await settle(tester);

      expect(ports.listeners.single.stops, 1);
      expect(ports.specs.last.start, InstallStart.bank0);
      expect(find.text('Wellfed is running. Now claim it.'), findsOneWidget);
    });

    testWidgets('the replacement answering on the network picks up from bank 0', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.uncertain());
      ports.listeners.single.updates.add(const ListenUpdate.replacement(address: '192.168.1.40:4747'));
      await settle(tester);
      expect(ports.specs.last.start, InstallStart.bank0);
      expect(ports.specs.last.address, '192.168.1.40:4747');
      // A run of its own, with the listener's port let go.
      expect(ports.runs, hasLength(2));
      expect(ports.listeners.single.stops, 1);
    });

    testWidgets('stock coming back says it did not install, and a closed request may start over', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.uncertain());
      ports.listeners.single.updates.add(const ListenUpdate.stockStayed(resolved: true));
      await settle(tester);

      expect(find.text("It didn't install"), findsOneWidget);
      await installTap(tester, 'Start again');
      expect(find.text('Tell the feeder how to reach this phone'), findsOneWidget);
    });

    testWidgets('stock coming back with the request still open offers no new request', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.uncertain());
      ports.listeners.single.updates.add(const ListenUpdate.stockStayed(resolved: false));
      await settle(tester);

      expect(find.text('A new install has to wait'), findsOneWidget);
      expect(find.text("It didn't install"), findsNothing);
      expect(find.text('Start again'), findsNothing);
      await installTap(tester, 'Listen again');
      expect(ports.listeners, hasLength(2));
    });

    testWidgets('silence for the whole wait offers listening again, and never a resend', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.uncertain());
      ports.listeners.single.updates.add(const ListenUpdate.silent());
      await settle(tester);

      expect(find.text('Still no word from the feeder'), findsOneWidget);
      expect(find.text('Start again'), findsNothing);
      expect(find.text('Listen again'), findsOneWidget);
      expect(find.text('Look for it over Bluetooth'), findsOneWidget);
      expect(ports.releases, 1);
    });

    testWidgets('a listener that cannot run says why', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.uncertain());
      ports.listeners.single.updates.add(const ListenUpdate.failed(why: ListenFailure.other, detail: 'no enrollment'));
      await settle(tester);
      expect(find.text("We can't listen for the feeder"), findsOneWidget);
      expect(find.textContaining('no enrollment'), findsOneWidget);
    });

    testWidgets('a listener whose server cannot start names the port, with its words below', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.uncertain());
      ports.listeners.single.updates.add(
        const ListenUpdate.failed(why: ListenFailure.serverFailed, detail: 'address in use'),
      );
      await settle(tester);
      expect(find.text("We can't listen for the feeder"), findsOneWidget);
      expect(find.textContaining('Another app may hold the port'), findsOneWidget);
      expect(find.text('address in use'), findsOneWidget);
      expect(find.text('Listen again'), findsOneWidget);
    });

    testWidgets('a listener that finds a damaged ledger offers to set it aside', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.uncertain());
      ports.pendingInstall = PendingInstall.ledgerDamaged;
      ports.listeners.single.updates.add(
        const ListenUpdate.failed(why: ListenFailure.ledgerDamaged, detail: 'a stock ledger does not parse'),
      );
      await settle(tester);
      expect(find.textContaining("can't read an install request's record"), findsOneWidget);
      expect(find.byType(InstallSetAsideLedger), findsOneWidget);
    });

    testWidgets('a run that finds its image refused says why, and sends nothing', (tester) async {
      await installStopped(
        tester,
        FakeInstallPorts(),
        const StopCase.imageRefused(check: ImageCheck.mismatch(detail: 'the review hash differs')),
      );
      expect(find.text("This build's firmware doesn't check out."), findsOneWidget);
      expect(find.text('the review hash differs'), findsOneWidget);
    });

    testWidgets('different firmware', (tester) async {
      await installStopped(tester, FakeInstallPorts(), const StopCase.otherFirmware(version: '1.1.14'));
      expect(find.text('Different vendor firmware'), findsOneWidget);
      expect(find.textContaining('This feeder runs 1.1.14.'), findsOneWidget);
      // Listed once, not again among the others.
      expect(find.text('Two feeders answered'), findsOneWidget);
    });

    testWidgets('two feeders forget the trust and start over', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.twoFeeders(), resume: Resume.fromStart);
      expect(find.text('Two feeders answered'), findsOneWidget);
      await installTap(tester, 'Start again');
      expect(ports.specs.last.forgetEnrollment, isTrue);
    });

    testWidgets('a backup that does not match picks up from bank 0', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.backupMismatch(), resume: Resume.fromBank0);
      expect(find.text("The backup didn't match"), findsOneWidget);
      await installTap(tester, 'Pick up where it stopped');
      expect(ports.specs.last.start, InstallStart.bank0);
    });

    testWidgets("not the owner's feeder forgets the trust", (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.notYourFeeder(), resume: Resume.fromStart);
      expect(find.text("That wasn't your feeder"), findsOneWidget);
      await installTap(tester, 'Start again');
      expect(ports.specs.last.forgetEnrollment, isTrue);
    });

    testWidgets('an open request can be listened for', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.earlierRequestOpen(), resume: Resume.fromStart);
      expect(find.text('An earlier install is still open'), findsOneWidget);
      await installTap(tester, 'Listen for it');
      expect(ports.listeners, hasLength(1));
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('an unset clock starts again', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.clockUnset(), resume: Resume.fromStart);
      expect(find.text("The feeder's clock isn't set"), findsOneWidget);
      await installTap(tester, 'Start again');
      expect(ports.specs, hasLength(2));
    });

    for (final (stop, title, resume, button) in [
      (const StopCase.noFeederCame(), 'No feeder connected', Resume.fromStart, 'Start again'),
      (const StopCase.stockRefused(code: 7), 'The feeder refused the install', Resume.fromStart, 'Start again'),
      (
        const StopCase.noSetupFeeder(),
        'No feeder is waiting over Bluetooth',
        Resume.fromBank0,
        'Pick up where it stopped',
      ),
      (
        const StopCase.setupFailed(refusal: SetupRefusal.wrongCode),
        "This phone couldn't claim it",
        Resume.fromBank0,
        'Pick up where it stopped',
      ),
      (const StopCase.setupFailed(), "This phone couldn't claim it", Resume.fromBank0, 'Pick up where it stopped'),
      (const StopCase.notJoined(), "It didn't join your Wi-Fi", Resume.fromBank0, 'Enter the Wi-Fi again'),
      (const StopCase.otherFeeder(), 'A different feeder answered', Resume.fromBank0, 'Pick up where it stopped'),
      (const StopCase.notAnswering(), 'The feeder stopped answering', Resume.fromBank0, 'Pick up where it stopped'),
      (
        const StopCase.promotionFailed(),
        "Wellfed didn't become permanent",
        Resume.fromBank0,
        'Pick up where it stopped',
      ),
      (const StopCase.fellBack(), 'It went back after restarting', Resume.fromBank0, 'Pick up where it stopped'),
      (
        const StopCase.uncertainWrite(write: UncertainWrite.promotion),
        "We don't know if that last step ran",
        Resume.fromBank0,
        'Pick up where it stopped',
      ),
      (const StopCase.serverFailed(), "This phone couldn't serve the feeder", Resume.fromStart, 'Start again'),
      (const StopCase.cancelled(), 'You stopped the install', Resume.fromStart, 'Start again'),
      (const StopCase.other(), 'The install stopped', Resume.nothing, 'Go to Home'),
    ]) {
      testWidgets('$title offers "$button"', (tester) async {
        await installStopped(tester, FakeInstallPorts(), stop, resume: resume);
        expect(find.text(title), findsOneWidget);
        expect(find.text(button), findsOneWidget);
        // The run's own words stay in the small print.
        expect(find.text('its words'), findsOneWidget);
      });
    }

    group('a request with no Wi-Fi to give', () {
      /// A fresh launch: an open request listened for, and stock staying.
      Future<void> stockStayed(WidgetTester tester, FakeInstallPorts ports) async {
        await installOpen(tester, ports);
        await installTap(tester, 'Listen for the feeder');
        ports.listeners.single.updates.add(const ListenUpdate.stockStayed(resolved: true));
        await settle(tester);
        expect(find.text('Start again'), findsOneWidget);
      }

      testWidgets('Start again after a fresh launch asks for the Wi-Fi', (tester) async {
        final ports = FakeInstallPorts(pendingInstall: PendingInstall.requestOpen);
        await stockStayed(tester, ports);

        await installTap(tester, 'Start again');
        expect(ports.runs, isEmpty);
        expect(find.text('Connect it to your Wi-Fi'), findsOneWidget);

        await tester.enterText(find.byType(TextField).at(0), 'Maple St');
        await tester.enterText(find.byType(TextField).at(1), 'correcthorse');
        await tester.pump();
        await installTap(tester, 'Continue');
        await installHoldReset(tester);
        expect(ports.specs.single.home?.ssid, 'Maple St');
        expect(ports.specs.single.start, InstallStart.reset);
      });
    });

    group('a feeder that never joined', () {
      Future<FakeRun> notJoined(WidgetTester tester, FakeInstallPorts ports) =>
          installStopped(tester, ports, const StopCase.notJoined(), resume: Resume.fromBank0);

      testWidgets('a new Wi-Fi goes through setup again, and says how', (tester) async {
        final ports = FakeInstallPorts();
        await notJoined(tester, ports);
        expect(find.text("It didn't join your Wi-Fi"), findsOneWidget);

        await installTap(tester, 'Enter the Wi-Fi again');
        expect(find.text('Connect it to your Wi-Fi'), findsOneWidget);
        await tester.enterText(find.byType(TextField).at(0), 'Maple St');
        await tester.enterText(find.byType(TextField).at(1), 'rightpassword');
        await tester.pump();
        await installTap(tester, 'Continue');

        expect(ports.specs, hasLength(2));
        final InstallSpec again = ports.specs.last;
        expect(again.start, InstallStart.bank0);
        expect(again.home?.passphrase, 'rightpassword');
        expect(again.unjoined, UnjoinedFeeder.setUpAgain);

        expect(find.text('Wellfed is running. Now claim it.'), findsOneWidget);
        expect(find.textContaining('Mic', findRichText: true), findsOneWidget);
      });

      testWidgets('a feeder paired with no record is picked up by identity', (tester) async {
        final ports = FakeInstallPorts();
        final FakeRun run = await installToRun(tester, ports);
        run.event(InstallEvent.paired(identity: Uint8List.fromList(const [0xab, 0xcd]), rebooting: false));
        run.stopped(const StopCase.other(), resume: Resume.fromBank0);
        await settle(tester);

        await installTap(tester, 'Pick up where it stopped');
        expect(ports.specs.last.start, InstallStart.bank0);
        expect(ports.specs.last.feeder, 'abcd');
        expect(ports.specs.last.unjoined, UnjoinedFeeder.wait);
      });

      testWidgets('a new Wi-Fi drops the pick, and setup runs again', (tester) async {
        final ports = FakeInstallPorts();
        final FakeRun run = await installToRun(tester, ports);
        run.event(InstallEvent.paired(identity: Uint8List.fromList(const [0xab, 0xcd]), rebooting: true));
        run.stopped(const StopCase.notJoined(), resume: Resume.fromBank0);
        await settle(tester);

        await installTap(tester, 'Keep waiting for it');
        // No record names it, so the run waits for the pick to join.
        expect(ports.specs.last.feeder, 'abcd');
        expect(ports.specs.last.unjoined, UnjoinedFeeder.waitForPick);
        ports.run.stopped(const StopCase.notJoined(), resume: Resume.fromBank0);
        await settle(tester);

        await installTap(tester, 'Enter the Wi-Fi again');
        await tester.enterText(find.byType(TextField).at(0), 'Maple St');
        await tester.enterText(find.byType(TextField).at(1), 'rightpassword');
        await tester.pump();
        await installTap(tester, 'Continue');

        expect(ports.specs.last.feeder, isNull);
        expect(ports.specs.last.unjoined, UnjoinedFeeder.setUpAgain);
      });

      testWidgets('a pick-up that finds no feeder keeps going through setup', (tester) async {
        final ports = FakeInstallPorts();
        await notJoined(tester, ports);
        await installTap(tester, 'Enter the Wi-Fi again');
        await tester.enterText(find.byType(TextField).at(0), 'Maple St');
        await tester.enterText(find.byType(TextField).at(1), 'rightpassword');
        await tester.pump();
        await installTap(tester, 'Continue');

        ports.run.stopped(const StopCase.noSetupFeeder(), resume: Resume.fromBank0);
        await settle(tester);
        await installTap(tester, 'Pick up where it stopped');
        expect(ports.specs.last.unjoined, UnjoinedFeeder.setUpAgain);
      });

      testWidgets('Back from the Wi-Fi leaves setup alone', (tester) async {
        final ports = FakeInstallPorts();
        await notJoined(tester, ports);
        await installTap(tester, 'Enter the Wi-Fi again');
        await tester.tap(find.byTooltip('Back'));
        await settle(tester);
        expect(find.text('Install Wellfed'), findsOneWidget);
        expect(ports.specs, hasLength(1));
      });

      testWidgets('waiting longer keeps the Wi-Fi it has', (tester) async {
        final ports = FakeInstallPorts();
        await notJoined(tester, ports);
        await installTap(tester, 'Keep waiting for it');
        expect(ports.specs.last.start, InstallStart.bank0);
        expect(ports.specs.last.unjoined, UnjoinedFeeder.waitForPick);
      });
    });

    group('what a run was given', () {
      testWidgets('a phone off Wi-Fi says so, not that the feeder stopped', (tester) async {
        final ports = FakeInstallPorts();
        await installStopped(tester, ports, const StopCase.phoneOffWifi(), resume: Resume.fromStart);
        expect(find.text("This phone isn't on Wi-Fi"), findsOneWidget);
        expect(find.text('The feeder stopped answering'), findsNothing);
        await installTap(tester, 'Start again');
        expect(find.text('Tell the feeder how to reach this phone'), findsOneWidget);
      });

      for (final (stop, title) in [
        (const StopCase.wifiRefused(), "That Wi-Fi can't be given to a feeder"),
        (const StopCase.noWifiGiven(), 'This install needs your Wi-Fi'),
      ]) {
        testWidgets(' asks for it again, and goes on from there', (tester) async {
          final ports = FakeInstallPorts();
          await installStopped(tester, ports, stop, resume: Resume.fromStart);
          expect(find.text(title), findsOneWidget);
          expect(find.text('The Bluetooth link dropped'), findsNothing);

          await installTap(tester, 'Enter the Wi-Fi again');
          expect(find.text('Connect it to your Wi-Fi'), findsOneWidget);
          await tester.enterText(find.byType(TextField).at(0), 'Maple St');
          await tester.enterText(find.byType(TextField).at(1), 'correcthorse');
          await tester.pump();
          await installTap(tester, 'Continue');
          expect(find.text('Tell the feeder how to reach this phone'), findsOneWidget);
        });
      }
    });

    for (final (why, title) in [
      (RequestRefusal.stale, "The feeder's reports got too old"),
      (RequestRefusal.offline, 'The feeder went offline'),
      (RequestRefusal.imageServer, "This phone wasn't ready to serve the firmware"),
      (RequestRefusal.other, "The install wasn't sent"),
    ]) {
      testWidgets('a refused request says why: $title', (tester) async {
        final ports = FakeInstallPorts();
        await installStopped(tester, ports, StopCase.requestRefused(why: why), resume: Resume.fromStart);
        expect(find.text(title), findsOneWidget);
        expect(find.text('An earlier install is still open'), findsNothing);
        await installTap(tester, 'Start again');
        expect(find.text('Tell the feeder how to reach this phone'), findsOneWidget);
      });
    }

    testWidgets('a stop closes the link, so the feeder advertises again', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToRun(tester, ports);
      run.ask(3, const InstallAsk.openLink());
      await settle(tester);
      expect(ports.link.closes, 0);

      run.stopped(const StopCase.setupFailed(), resume: Resume.fromBank0);
      await settle(tester);
      expect(ports.link.closes, 1);
    });

    testWidgets('a pick-up starts its code and checklist over', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToRun(tester, ports);
      run.event(const InstallEvent.setupStarting());
      run.event(InstallEvent.code(code: Uint8List.fromList(const [3, 1, 4, 4, 2, 1]), seconds: 60));
      await tester.pump();
      run.stopped(const StopCase.setupFailed(), resume: Resume.fromBank0);
      await settle(tester);

      await installTap(tester, 'Pick up where it stopped');
      expect(find.byType(CodeTiles), findsNothing);
      expect(find.text('Looking for the feeder on your Wi-Fi…'), findsOneWidget);
      await tester.pump(WaitWords.after);
      await tester.pump();
      expect(find.textContaining('Still looking for the feeder', findRichText: true), findsOneWidget);
    });

    testWidgets('a stream that closes with no end is a stop, not a hang', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToRun(tester, ports);
      unawaited(run.updates.close());
      await settle(tester);
      expect(find.text('The install stopped'), findsOneWidget);
      expect(find.text('the install stopped answering'), findsOneWidget);
    });

    testWidgets('a listener that closes with no end says so', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.uncertain());
      unawaited(ports.listeners.single.updates.close());
      await settle(tester);
      expect(find.text("We can't listen for the feeder"), findsOneWidget);
    });

    testWidgets('a stop with a paired feeder saves it for Home', (tester) async {
      final ports = FakeInstallPorts();
      final book = FakeBook();
      final FakeRun run = await installToRun(tester, ports, book: book);
      run.event(InstallEvent.paired(identity: Uint8List.fromList(const [0xab, 0xcd]), rebooting: false));
      await settle(tester);
      expect(book.feeders, isEmpty, reason: 'Home stays out of the run');

      run.stopped(const StopCase.promotionFailed(), resume: Resume.fromBank0);
      await settle(tester);
      expect(book.feeders.single.identity, 'abcd');
    });
  });
}
