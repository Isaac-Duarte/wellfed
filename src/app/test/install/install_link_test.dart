import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/src/rust/api/install.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';
import 'install_support.dart';

/// The claim's Bluetooth link: a feeder still linked does not advertise, so
/// every link the flow opened is hung up, whenever the run or the screen ends.
void main() {
  for (final (how, end) in [
    ('the run ends', (WidgetTester tester, FakeRun run) async => run.stopped(const StopCase.cancelled())),
    ('the screen goes', (WidgetTester tester, FakeRun run) => tester.pumpWidget(const SizedBox())),
  ]) {
    testWidgets('a link that opens after $how is hung up, so the feeder advertises again', (tester) async {
      final ports = FakeInstallPorts()..linkGate = Completer<void>();
      final FakeRun run = await installAtClaim(tester, ports);
      run.ask(3, const InstallAsk.openLink());
      await settle(tester);

      await end(tester, run);
      await settle(tester);
      ports.linkGate?.complete();
      await tester.pump();
      expect(ports.link.closes, 1);
      expect(run.answers[3], isNull, reason: 'nobody is left to answer');
      await tester.pumpWidget(const SizedBox());
    });
  }

  testWidgets('with Bluetooth off the claim says to turn it on, and the run looks again once it is', (tester) async {
    final ports = FakeInstallPorts()..power = BluetoothPower.off;
    final FakeRun run = await installAtClaim(tester, ports);
    run.ask(3, const InstallAsk.openLink());
    await settle(tester);

    // Nothing scanned: the run asks again on its own.
    expect(run.answers[3], isA<InstallAnswer_Failed>());
    expect(ports.linksOpened, 0);
    expect(find.textContaining('Bluetooth is off.', findRichText: true), findsOneWidget);
    await tester.ensureVisible(find.text('Turn on Bluetooth'));
    await tester.tap(find.text('Turn on Bluetooth'));
    await settle(tester);
    expect(ports.turnOnAsks, 1);

    ports.setPower(BluetoothPower.on);
    await settle(tester);
    expect(find.textContaining('Bluetooth is off.', findRichText: true), findsNothing);
    run.ask(4, const InstallAsk.openLink());
    await settle(tester);
    expect(run.answers[4], const InstallAnswer.done());
    expect(ports.linksOpened, 1);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a run that gave up with Bluetooth off says so where it stopped', (tester) async {
    final ports = FakeInstallPorts()..power = BluetoothPower.off;
    final FakeRun run = await installAtClaim(tester, ports);
    run.ask(3, const InstallAsk.openLink());
    await settle(tester);
    run.stopped(const StopCase.noSetupFeeder(), resume: Resume.fromBank0);
    await settle(tester);
    expect(find.textContaining('Bluetooth is off.', findRichText: true), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a link lost as Bluetooth goes off says Bluetooth is off', (tester) async {
    final ports = FakeInstallPorts();
    final FakeRun run = await installAtClaim(tester, ports);
    run.ask(3, const InstallAsk.openLink());
    await settle(tester);
    expect(run.answers[3], const InstallAnswer.done());

    ports.setPower(BluetoothPower.off);
    ports.onLost?.call();
    await tester.pump(SetupLink.powerSettle);
    await settle(tester);
    expect(run.lost, 1);
    expect(find.textContaining('Bluetooth is off.', findRichText: true), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a screen that goes while the plugin answers leaves nothing listening', (tester) async {
    final ports = FakeInstallPorts();
    final FakeRun run = await installAtClaim(tester, ports);
    ports.power = null;
    run.ask(3, const InstallAsk.openLink());
    await tester.pump();
    await tester.pumpWidget(const SizedBox());
    ports.setPower(BluetoothPower.off);
    await tester.pump(SetupLink.powerWait);
    expect(ports.powerListeners, 0);
    expect(ports.linksOpened, 0);
  });

  testWidgets('a link write with no link open fails, not done', (tester) async {
    final ports = FakeInstallPorts();
    final FakeRun run = await installAtClaim(tester, ports);
    run.ask(
      4,
      InstallAsk.writeLink(
        pieces: [
          Uint8List.fromList(const [1, 2]),
        ],
      ),
    );
    await settle(tester);
    expect(run.answers[4], isA<InstallAnswer_Failed>());
    expect(ports.link.written, isEmpty);
    await tester.pumpWidget(const SizedBox());
  });
}
