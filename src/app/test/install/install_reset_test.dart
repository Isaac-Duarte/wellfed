import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/install/install.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';
import 'install_support.dart';

void main() {
  testWidgets('every stage draws in dark too', (tester) async {
    final ports = FakeInstallPorts();
    tester.view.physicalSize = const Size(1080, 6000);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(installApp(FakeBook(), ports, brightness: Brightness.dark));
    await settle(tester);
    expect(find.text('Install Wellfed'), findsOneWidget);

    await installTap(tester, 'Continue');
    await tester.enterText(find.byType(TextField).at(0), 'Maple St');
    await tester.enterText(find.byType(TextField).at(1), 'correcthorse');
    await tester.pump();
    await installTap(tester, 'Continue');
    await installHoldReset(tester);
    final FakeRun run = ports.run;

    run.ask(1, const InstallAsk.recognizeFeeder(device: 'AF', address: '192.168.1.40'));
    await settle(tester);
    expect(tester.takeException(), isNull);
    await installTap(tester, 'Yes, install Wellfed');
    expect(tester.takeException(), isNull);
    run.event(const InstallEvent.setupStarting());
    await settle(tester);
    expect(tester.takeException(), isNull);
    run.event(InstallEvent.paired(identity: Uint8List(2), rebooting: true));
    await settle(tester);
    expect(tester.takeException(), isNull);
    run.end(const InstallEnd.done());
    await settle(tester);
    expect(find.text('Your feeder runs Wellfed.'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('a pick-up past the backup still offers to save it', (tester) async {
    final ports = FakeInstallPorts(pendingInstall: PendingInstall.paired)..kept = '/data/kept.bin';
    await installOpen(tester, ports);
    await installTap(tester, 'Pick up where it stopped');
    final FakeRun run = ports.run;
    run.event(
      InstallEvent.paired(
        identity: Uint8List.fromList(const [0xab, 0xcd]),
        address: '192.168.1.40:4747',
        rebooting: false,
      ),
    );
    run.end(const InstallEnd.done());
    await settle(tester);
    await installTap(tester, 'Save the backup');
    expect(ports.shared, ['/data/kept.bin']);
  });

  testWidgets('a pick-up keeps the name the owner gave the feeder', (tester) async {
    final book = FakeBook();
    await book.remember(const KnownFeeder(identity: 'abcd', name: 'Kitchen'));
    final ports = FakeInstallPorts(pendingInstall: PendingInstall.paired);
    await installOpen(tester, ports, book: book);
    await installTap(tester, 'Pick up where it stopped');
    final FakeRun run = ports.run;
    run.event(InstallEvent.paired(identity: Uint8List.fromList(const [0xab, 0xcd]), rebooting: false));
    run.stopped(const StopCase.promotionFailed(), resume: Resume.fromBank0);
    await settle(tester);
    expect(book.feeders.single.name, 'Kitchen');
  });

  testWidgets('Android refusing the service says to stay on screen', (tester) async {
    final ports = FakeInstallPorts()..holds = false;
    final FakeRun run = await installToRun(tester, ports);
    run.event(const InstallEvent.requesting(build: 'ab'));
    await settle(tester);
    expect(
      find.textContaining("Android won't keep Wellfed running in the background.", findRichText: true),
      findsOneWidget,
    );
  });

  test('only one run at a time: a second start leaves the first alone', () async {
    final ports = FakeInstallPorts();
    final flow = InstallFlow(FakeBook(), ports);
    await flow.load();
    await flow.startHandover();
    await flow.startHandover();
    expect(ports.runs, hasLength(1));
    flow.dispose();
  });

  test('the flow reads the APK before it offers anything', () async {
    final ports = FakeInstallPorts(pendingInstall: PendingInstall.paired);
    final flow = InstallFlow(FakeBook(), ports);
    await flow.load();
    expect(flow.state, isA<InstallBefore>());
    expect(flow.mayStart, isTrue);
    expect(flow.state.pending, PendingInstall.paired);
    expect(flow.state.phoneIp, '192.168.1.23');
    flow.dispose();
  });

  test('a disposed flow stops its run', () async {
    final ports = FakeInstallPorts();
    final flow = InstallFlow(FakeBook(), ports);
    await flow.load();
    await flow.startHandover();
    final FakeRun run = ports.run;
    flow.dispose();
    expect(run.stops, 1);
    expect(ports.releases, 1);
    expect(flow.state, isA<InstallHandover>());
  });

  group('2.2 and 2.3 the reset route', () {
    testWidgets('Reset waits for the owner to understand what it erases', (tester) async {
      final ports = FakeInstallPorts();
      await installOpen(tester, ports);
      await installTap(tester, 'Continue');
      await tester.enterText(find.byType(TextField).at(0), 'Maple St');
      await tester.enterText(find.byType(TextField).at(1), 'correcthorse');
      await tester.pump();
      await installTap(tester, 'Continue');

      expect(find.text('Reset the feeder'), findsOneWidget);
      await tester.tap(find.text('I held Reset'));
      await settle(tester);
      expect(ports.runs, isEmpty);
    });

    testWidgets('the phone joins, sends once, comes back and waits', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToHandover(tester, ports);
      expect(ports.specs.single.start, InstallStart.reset);
      expect(ports.specs.single.home?.ssid, 'Maple St');

      // The broker listens first, then the owner joins.
      run.ask(2, const InstallAsk.joinFeederNetwork());
      await settle(tester);
      expect(run.answers[2], isNull);
      await installTap(tester, "Join the feeder's network");
      expect(ports.joined, 1);
      expect(run.answers[2], const InstallAnswer.done());

      run.ask(3, const InstallAsk.leaveFeederNetwork());
      await settle(tester);
      expect(ports.left, 1);
      expect(run.answers[3], const InstallAnswer.done());
      expect(find.text('Waiting for the feeder to join your Wi-Fi…'), findsOneWidget);
    });

    testWidgets('an empty picker lets the owner try again', (tester) async {
      final ports = FakeInstallPorts()..joinFails = 'no network';
      final FakeRun run = await installToHandover(tester, ports);
      run.ask(2, const InstallAsk.joinFeederNetwork());
      await settle(tester);

      await installTap(tester, "Join the feeder's network");
      expect(find.textContaining("This phone didn't join the feeder's network.", findRichText: true), findsOneWidget);
      expect(run.answers[2], isNull);

      ports.joinFails = null;
      await installTap(tester, "Join the feeder's network");
      expect(run.answers[2], const InstallAnswer.done());
    });

    testWidgets('coming back at another address fails the ask', (tester) async {
      final ports = FakeInstallPorts()..ipAfterLeaving = '192.168.1.99';
      final FakeRun run = await installToHandover(tester, ports);
      run.ask(3, const InstallAsk.leaveFeederNetwork());
      await settle(tester);
      expect(run.answers[3], isA<InstallAnswer_Failed>());
    });

    testWidgets("a run that ends on the feeder's network leaves it", (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToHandover(tester, ports);
      run.ask(2, const InstallAsk.joinFeederNetwork());
      await settle(tester);
      await installTap(tester, "Join the feeder's network");

      run.stopped(const StopCase.cancelled(), resume: Resume.fromStart);
      await settle(tester);
      expect(ports.left, 1);
    });

    testWidgets('a frame stock did not keep goes again with no second reset', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToHandover(tester, ports);
      run.stopped(const StopCase.frameNotTaken(), resume: Resume.fromStart);
      await settle(tester);
      expect(find.text("The feeder didn't reach this phone"), findsOneWidget);

      await installTap(tester, 'Try again');
      expect(ports.runs, hasLength(2));
      expect(ports.specs.last.start, InstallStart.reset);
      expect(find.text('Reset the feeder'), findsNothing);
    });

    testWidgets('a new password goes straight to the handover', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToHandover(tester, ports);
      run.stopped(const StopCase.frameNotTaken(), resume: Resume.fromStart);
      await settle(tester);

      await installTap(tester, 'Enter the Wi-Fi again');
      await tester.enterText(find.byType(TextField).at(0), 'Maple St');
      await tester.enterText(find.byType(TextField).at(1), 'betterhorse');
      await tester.pump();
      await installTap(tester, 'Continue');
      expect(ports.runs, hasLength(2));
      expect(ports.specs.last.home?.passphrase, 'betterhorse');
    });

    testWidgets('a stock that kept the frame is only waited for', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToHandover(tester, ports);
      run.ask(4, const InstallAsk.recognizeFeeder(device: 'AF', address: '192.168.1.40'));
      await settle(tester);
      run.stopped(const StopCase.serverFailed(), resume: Resume.fromStart);
      await settle(tester);

      await installTap(tester, 'Start again');
      expect(ports.specs.last.start, InstallStart.frameTaken);
    });

    testWidgets('a client the owner said no to took no frame of ours', (tester) async {
      final ports = FakeInstallPorts();
      final FakeRun run = await installToHandover(tester, ports);
      run.ask(4, const InstallAsk.recognizeFeeder(device: 'AF', address: '192.168.1.40'));
      await settle(tester);
      await installTap(tester, "That's not my feeder");
      run.stopped(const StopCase.notYourFeeder(), resume: Resume.fromStart);
      await settle(tester);

      await installTap(tester, 'Start again');
      expect(ports.specs.last.start, InstallStart.reset);
    });
  });
}
