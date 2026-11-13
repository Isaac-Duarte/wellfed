import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/restore.dart';
import 'package:wellfed/state/restore/restore.dart';

import '../support/fakes.dart';
import 'restore_support.dart';

void main() {
  group('after an uncertain end', () {
    Future<void> uncertain(WidgetTester tester, FakeRestorePorts ports) async {
      await restoreOpen(tester, ports);
      final FakeRestoreRun run = await restoreStarted(tester, ports);
      run.end(const RestoreEnd.uncertain(why: UncertainWhy.gone, detail: 'gone'));
      await tester.pump();
    }

    testWidgets('Wellfed on the network offers to restore again', (tester) async {
      final ports = FakeRestorePorts()..located = '10.0.0.9:7700';
      await uncertain(tester, ports);
      await tester.pump();

      expect(find.text('Kitchen is still running Wellfed'), findsOneWidget);
      expect(find.text('Restore again'), findsOneWidget);
      expect(find.textContaining('hold Reset', findRichText: true), findsNothing);
    });

    testWidgets("a second uncertain run looks again, and never opens on the first one's verdict", (tester) async {
      final ports = FakeRestorePorts()..located = '10.0.0.9:7700';
      await uncertain(tester, ports);
      await tester.pump();
      expect(find.text('Kitchen is still running Wellfed'), findsOneWidget);

      await tester.tap(find.text('Restore again'));
      await settle(tester);
      ports.located = null;
      final FakeRestoreRun second = await restoreStarted(tester, ports);
      second.end(const RestoreEnd.uncertain(why: UncertainWhy.gone, detail: 'gone'));
      await tester.pump();

      expect(find.text("We can't tell if it went through"), findsOneWidget);
      expect(find.text('Kitchen is still running Wellfed'), findsNothing);
      expect(find.text('Restore again'), findsNothing);

      await tester.pump(RestoreFlow.lookFor);
      await tester.pump();
      expect(find.text("It's probably on the original firmware"), findsOneWidget);
    });

    testWidgets("an earlier run's look that answers late decides nothing for the next run", (tester) async {
      final ports = FakeRestorePorts()
        ..located = '10.0.0.9:7700'
        ..reading = Completer<RestoreSeen>();
      final RestoreFlow flow = restoreFlow(ports);
      addTearDown(flow.dispose);
      await flow.load();
      flow.setUnderstood(true);
      await flow.start();
      ports.runs.last.end(const RestoreEnd.uncertain(why: UncertainWhy.gone, detail: 'gone'));
      await tester.pump();

      // The first look's read is out when setup is heard, and the owner
      // starts again; the second look finds nothing on the network.
      ports.heard.add(null);
      await tester.pump();
      flow.again();
      flow.setUnderstood(true);
      ports.located = null;
      await flow.start();
      ports.runs.last.end(const RestoreEnd.uncertain(why: UncertainWhy.gone, detail: 'gone'));
      await tester.pump();

      final Completer<RestoreSeen>? first = ports.reading;
      ports.reading = null;
      first?.complete(RestoreSeen.idle);
      await tester.pump();
      expect(flow.state, isA<RestoreUncertain>().having((state) => state.look, 'look', RestoreLook.looking));

      await tester.pump(RestoreFlow.lookFor);
      expect(flow.state, isA<RestoreUncertain>().having((state) => state.look, 'look', RestoreLook.nothing));
    });

    testWidgets('a sighting that never reads idle says only that', (tester) async {
      final ports = FakeRestorePorts()
        ..located = '10.0.0.9:7700'
        ..seen = RestoreSeen.unread;
      await uncertain(tester, ports);
      await tester.pump();
      expect(find.text('Kitchen is still running Wellfed'), findsNothing);

      await tester.pump(RestoreFlow.lookFor);
      await tester.pump();

      expect(find.text("We still can't tell"), findsOneWidget);
      expect(find.text('Kitchen is still running Wellfed'), findsNothing);
      expect(find.text('Restore again'), findsNothing);
      expect(find.textContaining('hold Reset', findRichText: true), findsNothing);
    });

    testWidgets('a feeder in setup warns off Reset and offers setup', (tester) async {
      final ports = FakeRestorePorts();
      await uncertain(tester, ports);
      ports.heard.add(null);
      await tester.pump();

      expect(find.text('Set it up'), findsOneWidget);
      expect(find.textContaining("Don't hold Reset yet", findRichText: true), findsOneWidget);
    });

    testWidgets('only with nothing found does it speak of Reset', (tester) async {
      final ports = FakeRestorePorts();
      await uncertain(tester, ports);
      await tester.pump();
      expect(find.textContaining('hold Reset', findRichText: true), findsNothing, reason: 'not while it looks');

      await tester.pump(RestoreFlow.lookFor);
      await tester.pump();

      expect(find.text("It's probably on the original firmware"), findsOneWidget);
      expect(find.textContaining('hold Reset', findRichText: true), findsOneWidget);
    });

    testWidgets('with Bluetooth off, nothing found is no verdict, and the look starts again once it is on', (
      tester,
    ) async {
      final ports = FakeRestorePorts()..power = BluetoothPower.off;
      await uncertain(tester, ports);
      await tester.pump();
      expect(find.text('Looking for it on your Wi-Fi…'), findsOneWidget);

      await tester.pump(RestoreFlow.lookFor);
      await tester.pump();
      expect(find.text('Bluetooth is off'), findsOneWidget);
      expect(find.text("It's probably on the original firmware"), findsNothing);
      expect(find.textContaining('hold Reset', findRichText: true), findsNothing);

      await tester.tap(find.text('Turn on Bluetooth'));
      await tester.pump();
      expect(ports.turnOnAsks, 1);
      ports.setPower(BluetoothPower.on);
      await tester.pump();
      expect(find.text('Looking for it on your Wi-Fi and over Bluetooth…'), findsOneWidget);
      ports.heard.add(null);
      await tester.pump();
      expect(find.text('Set it up'), findsOneWidget);
    });

    testWidgets('Bluetooth going off during the look is no verdict either, and back on it looks again', (tester) async {
      final ports = FakeRestorePorts();
      await uncertain(tester, ports);
      await tester.pump();
      expect(find.text('Looking for it on your Wi-Fi and over Bluetooth…'), findsOneWidget);

      ports.setPower(BluetoothPower.off);
      await tester.pump(RestoreFlow.lookFor);
      await tester.pump();
      expect(find.text('Bluetooth is off'), findsOneWidget);
      expect(find.textContaining('hold Reset', findRichText: true), findsNothing);

      ports.setPower(BluetoothPower.on);
      await tester.pump();
      await tester.pump();
      expect(find.text('Looking for it on your Wi-Fi and over Bluetooth…'), findsOneWidget);
      await tester.pump(RestoreFlow.lookFor);
      await tester.pump();
      expect(find.text("It's probably on the original firmware"), findsOneWidget);
    });
  });

  testWidgets('leaving after a stopped run forgets the key', (tester) async {
    final ports = FakeRestorePorts();
    await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);
    run.stopped(RestoreStop.notAnswering);
    await settle(tester);
    expect(ports.forgotten, 0, reason: 'Try again still wants it');

    await tester.pumpWidget(const SizedBox.shrink());
    expect(ports.forgotten, 1);
  });

  testWidgets('a second tap during the hold starts nothing more', (tester) async {
    final ports = FakeRestorePorts()..holding = Completer<void>();
    final RestoreFlow flow = await restoreOpen(tester, ports);
    flow.setUnderstood(true);

    unawaited(flow.start());
    unawaited(flow.start());
    await tester.pump();
    expect(ports.held, hasLength(1));

    ports.holding!.complete();
    await tester.pump();
    expect(ports.runs, hasLength(1));
    ports.runs.single.stopped(RestoreStop.cancelled);
    await settle(tester);
  });

  testWidgets('leaving during the hold lets the process go once it takes', (tester) async {
    final ports = FakeRestorePorts()..holding = Completer<void>();
    final RestoreFlow flow = await restoreOpen(tester, ports);
    flow.setUnderstood(true);
    unawaited(flow.start());
    await tester.pump();

    await tester.pumpWidget(const SizedBox.shrink());
    final int before = ports.released;
    ports.holding!.complete();
    await tester.pump();

    expect(ports.released, before + 1);
    expect(ports.runs, isEmpty);
  });

  for (final Brightness brightness in Brightness.values) {
    testWidgets('every stage draws in $brightness', (tester) async {
      final ports = FakeRestorePorts();
      await restoreOpen(tester, ports, brightness: brightness);
      expect(tester.takeException(), isNull);

      final FakeRestoreRun run = await restoreStarted(tester, ports);
      run.event(const InstallEvent.pressDispense());
      await settle(tester);
      expect(tester.takeException(), isNull);

      run.event(const InstallEvent.promoting());
      run.end(const RestoreEnd.activated());
      await settle(tester);
      expect(tester.takeException(), isNull);
    });
  }
}
