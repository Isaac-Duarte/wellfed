import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
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
  });

  testWidgets('leaving after a stopped run forgets the key', (tester) async {
    final ports = FakeRestorePorts();
    final RestoreFlow flow = await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);
    run.stopped(RestoreStop.notAnswering);
    await settle(tester);
    expect(ports.forgotten, 0, reason: 'Try again still wants it');

    await tester.pumpWidget(const SizedBox.shrink());
    flow.dispose();
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
    flow.dispose();
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
