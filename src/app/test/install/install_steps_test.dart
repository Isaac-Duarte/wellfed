import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/widgets/code_tiles.dart';
import 'package:wellfed/widgets/layout.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';
import 'install_support.dart';

void main() {
  group('2.1 before you start', () {
    testWidgets('it says what it takes and offers no choice of route', (tester) async {
      final ports = FakeInstallPorts();
      await installOpen(tester, ports);

      expect(find.text('Install Wellfed'), findsOneWidget);
      expect(find.textContaining('Through'), findsNothing);
    });

    for (final (image, words) in [
      (const ImageCheck.absent(), 'This build has no firmware to install.'),
      (const ImageCheck.notPinned(), 'This build pins no firmware.'),
      (
        const ImageCheck.mismatch(detail: 'review.json is not the pinned one'),
        "This build's firmware doesn't check out.",
      ),
    ]) {
      testWidgets('an APK with no usable image says so: $words', (tester) async {
        await installOpen(tester, FakeInstallPorts(image: image));
        expect(find.textContaining(words, findRichText: true), findsOneWidget);
        final FilledButton button = tester.widget<FilledButton>(find.byType(FilledButton));
        expect(button.onPressed, isNull);
      });
    }

    testWidgets('a build of unclean sources cannot send an install', (tester) async {
      final ports = FakeInstallPorts(build: const BuildCheck.unreviewed(commit: 'abc1234'));
      await installOpen(tester, ports);
      expect(find.textContaining('This build cannot send an install.', findRichText: true), findsOneWidget);
      final FilledButton button = tester.widget<FilledButton>(find.byType(FilledButton));
      expect(button.onPressed, isNull);
    });

    testWidgets('Bluetooth refused stays here and says what to allow', (tester) async {
      final ports = FakeInstallPorts()..allowed = false;
      await installOpen(tester, ports);
      await installTap(tester, 'Continue');
      expect(find.text('Nearby devices is off'), findsOneWidget);
    });

    testWidgets('an unnamed phone is named first', (tester) async {
      final book = FakeBook(label: '');
      await installOpen(tester, FakeInstallPorts(), book: book);
      await installTap(tester, 'Continue');
      expect(find.text('Name this phone'), findsOneWidget);
      await tester.enterText(find.byType(TextField).first, 'Pixel');
      await tester.pump();
      await installTap(tester, 'Continue');
      expect(book.label, 'Pixel');
      expect(find.text('Connect it to your Wi-Fi'), findsOneWidget);
    });

    testWidgets('a pending install picks up from bank 0, asking stock nothing', (tester) async {
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.paired);
      await installOpen(tester, ports);
      await installTap(tester, 'Pick up where it stopped');
      expect(ports.specs.single.start, InstallStart.bank0);
      // Paired: no Wi-Fi to give it.
      expect(ports.specs.single.home, isNull);
      expect(find.text('Wellfed is running. Now claim it.'), findsOneWidget);
    });

    testWidgets('an open request offers the listener, and never a new request', (tester) async {
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.requestOpen);
      await installOpen(tester, ports);
      expect(find.textContaining('An earlier install has not finished.', findRichText: true), findsOneWidget);
      expect(find.text('Pick up where it stopped'), findsNothing);

      await installTap(tester, 'Listen for the feeder');
      expect(ports.runs, isEmpty);
      expect(ports.listeners, hasLength(1));
      // It may have installed and gone back since, so it claims nothing.
      expect(find.text('Waiting to hear from the feeder'), findsOneWidget);
      expect(find.text("We don't know yet if it installed"), findsNothing);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('looking over Bluetooth after a restart asks for the Wi-Fi first', (tester) async {
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.requestOpen);
      await installOpen(tester, ports);
      await installTap(tester, 'Look for it over Bluetooth');
      expect(ports.runs, isEmpty);
      expect(find.text('Connect it to your Wi-Fi'), findsOneWidget);

      await tester.enterText(find.byType(TextField).at(0), 'Maple St');
      await tester.enterText(find.byType(TextField).at(1), 'correcthorse');
      await tester.pump();
      await installTap(tester, 'Continue');
      expect(ports.specs.single.start, InstallStart.bank0);
      expect(ports.specs.single.home?.ssid, 'Maple St');
      expect(find.text('Wellfed is running. Now claim it.'), findsOneWidget);
    });

    testWidgets('Back from the Wi-Fi a pick-up asked for drops that pick-up', (tester) async {
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.requestOpen);
      await installOpen(tester, ports);
      await installTap(tester, 'Look for it over Bluetooth');
      expect(find.text('Connect it to your Wi-Fi'), findsOneWidget);

      await tester.tap(find.byTooltip('Back'));
      await settle(tester);
      await installTap(tester, 'Continue');
      await tester.enterText(find.byType(TextField).at(0), 'Maple St');
      await tester.enterText(find.byType(TextField).at(1), 'correcthorse');
      await tester.pump();
      await installTap(tester, 'Continue');

      // The owner's Continue is the first screen's: on to the Reset hold, with no
      // run from bank 0.
      expect(ports.runs, isEmpty);
      expect(find.text('Reset the feeder'), findsOneWidget);
    });

    testWidgets('every way on from the first screen asks Android first', (tester) async {
      for (final button in ['Listen for the feeder', 'Look for it over Bluetooth']) {
        final ports = FakeInstallPorts(pendingInstall: PendingInstall.requestOpen)..allowed = false;
        await installOpen(tester, ports);
        await installTap(tester, button);
        expect(ports.asked, 1, reason: button);
        expect(ports.runs, isEmpty, reason: button);
        expect(ports.listeners, isEmpty, reason: button);
        expect(find.textContaining('Nearby devices is off.', findRichText: true), findsOneWidget, reason: button);
        await tester.pumpWidget(const SizedBox());
        await settle(tester);
      }

      final ports = FakeInstallPorts(pendingInstall: PendingInstall.paired)..allowed = false;
      await installOpen(tester, ports);
      await installTap(tester, 'Pick up where it stopped');
      expect(ports.asked, 1);
      expect(ports.runs, isEmpty);
    });

    testWidgets('an unnamed phone is named before a pick-up, which goes on', (tester) async {
      final book = FakeBook(label: '');
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.paired);
      await installOpen(tester, ports, book: book);
      await installTap(tester, 'Pick up where it stopped');
      expect(find.text('Name this phone'), findsOneWidget);
      await tester.enterText(find.byType(TextField).first, 'Pixel');
      await tester.pump();
      await installTap(tester, 'Continue');
      expect(ports.specs.single.label, 'Pixel');
      expect(ports.specs.single.start, InstallStart.bank0);
    });
  });

  group('2.3 the wait', () {
    testWidgets('the run starts with this phone, and is held for the run', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToRun(tester, ports);

      final InstallSpec spec = ports.specs.single;
      expect(spec.start, InstallStart.reset);
      expect(spec.phoneIp, '192.168.1.23');
      expect(spec.home?.ssid, 'Maple St');
      expect(spec.label, 'Pixel');

      // The screen says to restart it; the broker is already listening.
      run.ask(2, const InstallAsk.restartFeeder());
      await settle(tester);
      expect(run.answers[2], const InstallAnswer.done());
      expect(ports.held, isNotEmpty);
    });

    testWidgets('why it still waits, and a lost report asks for one more restart', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToRun(tester, ports);

      run.event(const InstallEvent.waiting(why: WaitReason.notOnAdapter));
      await settle(tester);
      expect(find.textContaining('on batteries'), findsOneWidget);

      run.event(const InstallEvent.versionLost());
      await settle(tester);
      expect(find.textContaining('Restart the feeder once more:', findRichText: true), findsOneWidget);
    });

    testWidgets('leaving a run asks first, and stops it through its cleanup', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToRun(tester, ports);
      await tester.tap(find.byTooltip('Close'));
      await settle(tester);
      expect(find.text('Stop the install?'), findsOneWidget);
      await installTap(tester, 'Stop');
      expect(run.stops, 1);
    });
  });

  group('2.4 is this your feeder', () {
    testWidgets('it shows what connected, and yes goes on to the install', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installAtConfirm(tester, ports);

      expect(find.text('A feeder connected. Is it yours?'), findsOneWidget);
      expect(find.text('AF_TEST'), findsOneWidget);
      expect(find.text('192.168.1.40'), findsOneWidget);
      expect(find.text('Vendor firmware 1.1.15'), findsOneWidget);

      await installTap(tester, 'Yes, install Wellfed');
      expect(run.answers[9], const InstallAnswer.yes(yes: true));
      expect(find.text('Installing Wellfed'), findsOneWidget);
    });

    testWidgets('no answers no, and nothing was sent', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installAtConfirm(tester, ports);
      await installTap(tester, "That's not my feeder");
      expect(run.answers[9], const InstallAnswer.yes(yes: false));
    });
  });

  group('2.5 installing', () {
    testWidgets('progress, the time left, and the restart', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToRun(tester, ports);

      run.event(const InstallEvent.requesting(build: 'ab'));
      run.event(const InstallEvent.stockProgress(percent: 62));
      await settle(tester);
      expect(find.text('Installing Wellfed'), findsOneWidget);
      expect(find.text('Step 3 of 5'), findsOneWidget);
      expect(find.text('Sending firmware'), findsOneWidget);
      expect(find.textContaining('62%'), findsOneWidget);
      expect(find.textContaining('Keep Wellfed open.', findRichText: true), findsOneWidget);
      expect(ports.held.last, contains('serving the firmware'));

      run.event(const InstallEvent.stockProgress(percent: 100));
      await settle(tester);
      await tester.pump(WaitWords.after);
      await tester.pump();
      expect(find.textContaining('is installing it', findRichText: true), findsOneWidget);
      expect(find.textContaining('next part'), findsNothing);

      run.event(const InstallEvent.stockRestarting());
      await settle(tester);
      expect(find.text('Restarting into Wellfed'), findsOneWidget);
      expect(find.textContaining('%'), findsNothing, reason: 'the send is done');
    });
  });

  group('2.6 claim', () {
    testWidgets('a consumer image is found over Bluetooth, with the code as setup shows it', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installAtClaim(tester, ports);
      expect(find.text('Wellfed is running. Now claim it.'), findsOneWidget);
      expect(find.text('Looking for the feeder over Bluetooth…'), findsOneWidget);

      run.ask(3, const InstallAsk.openLink());
      await settle(tester);
      expect(run.answers[3], const InstallAnswer.done());

      run.ask(
        4,
        InstallAsk.writeLink(
          pieces: [
            Uint8List.fromList(const [1, 2]),
          ],
        ),
      );
      await settle(tester);
      expect(run.answers[4], const InstallAnswer.done());
      expect(ports.link.written.single, [
        [1, 2],
      ]);

      ports.onBytes!(const [5, 6]);
      expect(run.notifications.single, [5, 6]);

      run.event(InstallEvent.code(code: Uint8List.fromList(const [3, 1, 4, 4, 2, 1]), seconds: 60));
      await tester.pump();
      expect(find.byType(CodeTiles), findsOneWidget);
      expect(find.bySemanticsLabel('Press Mic, Lock, Reset, Reset, Dispense, Lock'), findsOneWidget);
      expect(find.textContaining(RegExp(r'^(1:00|0:5\d)$')), findsOneWidget);
      expect(find.text('Waiting for the feeder to confirm…'), findsOneWidget);

      run.ask(5, const InstallAsk.closeLink());
      await tester.pump();
      expect(run.answers[5], const InstallAnswer.done());
      expect(ports.link.closes, 1);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('a radio that finds nothing tells the run, which tries again', (tester) async {
      final ports = FakeInstallPorts()..linkFails = 'no feeder in time';
      final FakeRun run = await installAtClaim(tester, ports);
      run.ask(3, const InstallAsk.openLink());
      await settle(tester);
      expect(run.answers[3], const InstallAnswer.failed(reason: 'no feeder in time'));
    });

    testWidgets('a dropped link reaches the run', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installAtClaim(tester, ports);
      run.ask(3, const InstallAsk.openLink());
      await settle(tester);
      ports.onLost!();
      expect(run.lost, 1);
    });

    testWidgets('NSD answers the run', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installAtClaim(tester, ports);
      run.ask(6, const InstallAsk.locate(identity: 'abcd'));
      await settle(tester);
      expect(run.answers[6], const InstallAnswer.located(address: '192.168.1.40:4747'));
    });

    testWidgets('a bench image asks for its recovery window, and the owner says when', (tester) async {
      final ports = FakeInstallPorts(
        image: const ImageCheck.ready(provisioning: ImageProvisioning.bench, build: 'ab', version: 7),
      );
      await installOpen(tester, ports);
      await installTap(tester, 'Continue');
      await tester.enterText(find.byType(TextField).at(0), 'Maple St');
      await tester.enterText(find.byType(TextField).at(1), 'correcthorse');
      await tester.pump();
      await installTap(tester, 'Continue');
      await installHoldReset(tester);
      final FakeRun run = ports.run;

      run.ask(7, const InstallAsk.openRecoveryWindow());
      await settle(tester);
      expect(find.textContaining('then also the', findRichText: true), findsOneWidget);
      await installTap(tester, 'The lock light is blinking fast');
      expect(run.answers[7], const InstallAnswer.done());
    });
  });

  group('long waits', () {
    testWidgets('no find over Bluetooth asks for the hold, until a code comes', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installAtClaim(tester, ports);
      Finder hint() => find.textContaining('Hold Lock + Mic for 5 s', findRichText: true);
      expect(hint(), findsNothing);

      await tester.pump(InstallFlow.setupHintAfter);
      expect(hint(), findsOneWidget);
      expect(find.textContaining('If the lock light is on', findRichText: true), findsOneWidget);

      run.event(InstallEvent.code(code: Uint8List.fromList(const [1, 2]), seconds: 60));
      await tester.pump();
      expect(hint(), findsNothing);
    });

    testWidgets('joining says what it waits for, and only once it is long', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installAtFinishing(tester, ports);
      Finder words() => find.textContaining('restart and join your Wi-Fi', findRichText: true);
      expect(words(), findsNothing);

      await tester.pump(WaitWords.after);
      await tester.pump();
      expect(words(), findsOneWidget);

      run.event(const InstallEvent.joined(address: '192.168.1.40:4747'));
      await tester.pump();
      expect(words(), findsNothing);
    });

    testWidgets('copying goes quiet only when its progress stops', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installAtFinishing(tester, ports);
      run.event(const InstallEvent.joined(address: '192.168.1.40:4747'));
      run.event(const InstallEvent.readingBackup(total: 1000));
      await tester.pump();
      Finder words() => find.textContaining('Waiting for the feeder to answer', findRichText: true);

      await tester.pump(WaitWords.after - const Duration(seconds: 1));
      run.event(InstallEvent.backupRead(bytes: BigInt.from(500), total: 1000));
      await tester.pump();
      await tester.pump(WaitWords.after - const Duration(seconds: 1));
      expect(words(), findsNothing);

      await tester.pump(const Duration(seconds: 2));
      await tester.pump();
      expect(words(), findsOneWidget);
    });
  });

  group('2.7 back up and finish', () {
    testWidgets('each step in its turn', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installAtFinishing(tester, ports);

      expect(find.text('Backing up the original firmware'), findsOneWidget);
      expect(find.text('This phone is its owner'), findsOneWidget);
      expect(find.text('Finding it on your network'), findsOneWidget);

      run.event(const InstallEvent.joined(address: '192.168.1.40:4747'));
      run.event(const InstallEvent.readingBackup(total: 1000));
      run.event(InstallEvent.backupRead(bytes: BigInt.from(750), total: 1000));
      await tester.pump();
      expect(find.text('75%'), findsOneWidget);

      run.event(const InstallEvent.backupKept(path: '/data/stock.bin'));
      run.event(const InstallEvent.promoting());
      run.event(const InstallEvent.written(written: 500, length: 1000));
      await tester.pump();
      expect(find.text('Making Wellfed permanent'), findsOneWidget);
      expect(find.text('50%'), findsOneWidget);

      run.event(const InstallEvent.activated());
      await tester.pump();
      expect(find.text('Restart and confirm'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    });
  });

  group('2.8 done', () {
    Future<(FakeRun, FakeBook)> finished(WidgetTester tester, FakeInstallPorts ports) async {
      final book = FakeBook();
      final FakeRun run = await installToRun(tester, ports, book: book);
      run.event(InstallEvent.paired(identity: Uint8List.fromList(const [0xab, 0xcd]), rebooting: true));
      run.event(const InstallEvent.joined(address: '192.168.1.40:4747'));
      run.event(const InstallEvent.backupKept(path: '/data/stock.bin'));
      run.event(const InstallEvent.installed(build: 'ab'));
      run.end(const InstallEnd.done());
      await settle(tester);
      return (run, book);
    }

    testWidgets('it says to set up meals and remove the record, and names the feeder', (tester) async {
      final ports = FakeInstallPorts();
      final (_, FakeBook book) = await finished(tester, ports);

      expect(find.text('Your feeder runs Wellfed.'), findsOneWidget);
      expect(find.text('Set up its meals here next.'), findsOneWidget);
      expect(ports.releases, 1);

      // Home hears of it as the run ends, due its first clock sync.
      expect(book.remembered.first.$2, isTrue);

      await tester.enterText(find.byType(TextField), 'Kitchen');
      await installTap(tester, 'Go to the feeder');
      expect(find.text('Home'), findsOneWidget);
      final KnownFeeder saved = book.feeders.single;
      expect(saved.identity, 'abcd');
      expect(saved.name, 'Kitchen');
      expect(saved.address, '192.168.1.40:4747');
    });

    testWidgets('the backup goes to the share sheet', (tester) async {
      final ports = FakeInstallPorts();
      await finished(tester, ports);
      await installTap(tester, 'Save the backup somewhere else');
      expect(ports.shared, ['/data/stock.bin']);
    });
  });
}
