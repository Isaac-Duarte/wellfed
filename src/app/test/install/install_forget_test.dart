import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/setup.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/install/install.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';
import 'install_support.dart';

void main() {
  group('forgetting an install', () {
    Future<void> tapForget(WidgetTester tester) async {
      await tester.ensureVisible(find.text('Forget this install'));
      await settle(tester);
      await tester.tap(find.text('Forget this install'));
      await settle(tester);
    }

    testWidgets('a damaged record reaches it from the first screen', (tester) async {
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.damaged);
      await installOpen(tester, ports);

      expect(
        find.textContaining("This phone can't read its record of an earlier install.", findRichText: true),
        findsOneWidget,
      );
      final FilledButton go = tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'Continue'));
      expect(go.onPressed, isNull);

      await tapForget(tester);
      expect(find.text('Every backup of the original firmware stays on this phone.'), findsOneWidget);
      expect(find.textContaining('no known result'), findsNothing);
      await tester.tap(find.text('Forget'));
      await settle(tester);

      expect(ports.forgets, 1);
      expect(find.text('Forget this install'), findsNothing);
      expect(tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'Continue')).onPressed, isNotNull);
    });

    testWidgets('a damaged stop reaches it, not a dead end', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.damagedRecord());
      expect(find.text("This phone can't read its install record"), findsOne);

      await tapForget(tester);
      await tester.tap(find.text('Forget'));
      await settle(tester);

      expect(ports.forgets, 1);
      expect(find.text('Install Wellfed'), findsOneWidget);
    });

    testWidgets('an other-feeder stop offers it, and cancel clears nothing', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.otherFeeder(), resume: Resume.fromBank0);
      expect(find.textContaining('after a factory reset'), findsOneWidget);

      await tapForget(tester);
      await tester.tap(find.text('Cancel'));
      await settle(tester);

      expect(ports.forgets, 0);
      expect(find.text('A different feeder answered'), findsOneWidget);
    });

    testWidgets('an open request is warned of, and kept', (tester) async {
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.requestOpen)..openRequest = true;
      await installOpen(tester, ports);

      await tapForget(tester);
      expect(find.textContaining('no known result'), findsOneWidget);
      expect(find.textContaining('never sent it a second time'), findsOneWidget);
    });

    testWidgets('after it, a feeder on bank 0 goes on from the first screen', (tester) async {
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.paired);
      await installOpen(tester, ports);

      await tapForget(tester);
      expect(find.textContaining('ends here'), findsOneWidget);
      await tester.tap(find.text('Forget'));
      await settle(tester);
      expect(find.text('Pick up where it stopped'), findsNothing);

      await tester.tap(find.text('This feeder already runs Wellfed'));
      await settle(tester);
      // No record says the Wi-Fi was given, so a consumer image asks for it.
      expect(ports.runs, isEmpty);
      await tester.enterText(find.byType(TextField).first, 'Maple St');
      await tester.enterText(find.byType(TextField).last, 'correcthorse');
      await settle(tester);
      await tester.tap(find.widgetWithText(FilledButton, 'Continue'));
      await settle(tester);

      expect(ports.specs.single.start, InstallStart.bank0);
      expect(ports.specs.single.home?.ssid, 'Maple St');
    });

    group('with no record, which feeder', () {
      const kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');

      Future<(FakeInstallPorts, InstallFlow)> asked(WidgetTester tester) async {
        final ports = FakeInstallPorts();
        final book = FakeBook()..feeders = [kitchen];
        final flow = InstallFlow(book, ports);
        addTearDown(flow.dispose);
        await flow.load();
        flow.alreadyRuns();
        return (ports, flow);
      }

      testWidgets('nothing starts until the owner says', (tester) async {
        final (FakeInstallPorts ports, InstallFlow flow) = await asked(tester);
        expect(flow.stage, InstallStage.which);
        expect(ports.runs, isEmpty);
      });

      testWidgets('a picked feeder goes to the run, with no Wi-Fi to ask', (tester) async {
        final (FakeInstallPorts ports, InstallFlow flow) = await asked(tester);
        await flow.pickFeeder('aa');
        expect(ports.specs.single.feeder, 'aa');
        expect(ports.specs.single.start, InstallStart.bank0);
      });

      testWidgets('a picked feeder is held for the pick-up', (tester) async {
        final (FakeInstallPorts ports, InstallFlow flow) = await asked(tester);
        await flow.pickFeeder('aa');
        ports.run.stopped(const StopCase.other(), resume: Resume.fromBank0);
        await tester.pump();

        await flow.again();
        expect(ports.specs, hasLength(2));
        expect(ports.specs.last.feeder, 'aa');
      });

      testWidgets('another feeder after a paired one still means Bluetooth', (tester) async {
        final (FakeInstallPorts ports, InstallFlow flow) = await asked(tester);
        await flow.pickFeeder('aa');
        ports.run.event(InstallEvent.paired(identity: Uint8List.fromList(const [0xaa]), rebooting: false));
        ports.run.stopped(const StopCase.other(), resume: Resume.fromBank0);
        await tester.pump();

        await flow.pickFeeder(null);
        expect(flow.stage, InstallStage.wifi);
      });

      testWidgets('another feeder names none, and asks for the Wi-Fi', (tester) async {
        final (FakeInstallPorts ports, InstallFlow flow) = await asked(tester);
        await flow.pickFeeder(null);
        expect(flow.stage, InstallStage.wifi);
        expect(ports.runs, isEmpty);

        flow.setHome(const WifiCredentials(ssid: 'Maple St', passphrase: 'correcthorse'));
        await tester.pump();
        expect(ports.specs.single.feeder, isNull);
      });
    });

    testWidgets('the screen lists the known feeders and another', (tester) async {
      final ports = FakeInstallPorts();
      final book = FakeBook()..feeders = [const KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747')];
      await tester.pumpWidget(installApp(book, ports));
      await settle(tester);
      await tester.ensureVisible(find.text('This feeder already runs Wellfed'));
      await settle(tester);
      await tester.tap(find.text('This feeder already runs Wellfed'));
      await settle(tester);

      expect(find.text('Which feeder?'), findsOneWidget);
      expect(find.text('Kitchen'), findsOneWidget);
      expect(find.text('Another feeder, over Bluetooth'), findsOneWidget);
      expect(ports.runs, isEmpty);

      await tester.tap(find.text('Kitchen'));
      await settle(tester);
      expect(ports.specs.single.feeder, 'aa');
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets("an uncertain write's stop does not offer it", (tester) async {
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.paired);
      await installStopped(
        tester,
        ports,
        const StopCase.uncertainWrite(write: UncertainWrite.promotion),
        resume: Resume.fromBank0,
      );
      expect(find.text('Forget this install'), findsNothing);
    });

    testWidgets('a working pick-up says it ends, and how to come back', (tester) async {
      for (final stop in const [
        StopCase.notAnswering(),
        StopCase.promotionFailed(),
        StopCase.fellBack(),
        StopCase.notConfirmed(),
      ]) {
        final ports = FakeInstallPorts(pendingInstall: PendingInstall.paired);
        await installStopped(tester, ports, stop, resume: Resume.fromBank0);
        await tapForget(tester);
        expect(find.textContaining('This feeder already runs Wellfed'), findsOneWidget, reason: '$stop');
        await tester.tap(find.text('Cancel'));
        await settle(tester);
        await tester.pumpWidget(const SizedBox());
      }
    });

    testWidgets("a damaged ledger takes the owner's word, in its own words", (tester) async {
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.ledgerDamaged);
      await installOpen(tester, ports);
      expect(find.text('Forget this install'), findsNothing);
      expect(tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'Continue')).onPressed, isNull);

      await tester.ensureVisible(find.text('Set the damaged record aside'));
      await settle(tester);
      await tester.tap(find.text('Set the damaged record aside'));
      await settle(tester);
      expect(find.textContaining('no longer knows whether an install'), findsOneWidget);
      expect(find.textContaining('Check that feeder first'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await settle(tester);
      expect(ports.ledgersSetAside, 0);

      await tester.tap(find.text('Set the damaged record aside'));
      await settle(tester);
      await tester.tap(find.text('Set it aside'));
      await settle(tester);
      expect(ports.ledgersSetAside, 1);
      expect(find.text('Set the damaged record aside'), findsNothing);
    });

    testWidgets('a damaged backup is set aside only by the owner', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.backupDamaged(path: 'kept.bin'), resume: Resume.fromBank0);
      expect(ports.backupsSetAside, isEmpty);

      await tester.ensureVisible(find.text('Set this backup aside'));
      await settle(tester);
      await tester.tap(find.text('Set this backup aside'));
      await settle(tester);
      expect(find.textContaining('can no longer be used to go back'), findsOneWidget);
      expect(find.textContaining('reads a fresh copy only if the feeder still'), findsOneWidget);
      await tester.tap(find.text('Set it aside'));
      await settle(tester);
      expect(ports.backupsSetAside, [('kept.bin', BackupProblem.damaged)]);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('it is not offered while listening', (tester) async {
      final ports = FakeInstallPorts()..pendingInstall = PendingInstall.paired;
      await installStopped(tester, ports, const StopCase.uncertain());
      expect(find.text('Forget this install'), findsNothing);
    });

    testWidgets('a failure says so and stays', (tester) async {
      final ports = FakeInstallPorts(pendingInstall: PendingInstall.damaged)
        ..forgetFails = const BridgeError(kind: ErrorKind.store, message: 'read-only');
      await installOpen(tester, ports);

      await tapForget(tester);
      await tester.tap(find.text('Forget'));
      await settle(tester);

      expect(find.textContaining('read-only', findRichText: true), findsOneWidget);
      expect(find.text('Forget this install'), findsOneWidget);
    });
  });
}
