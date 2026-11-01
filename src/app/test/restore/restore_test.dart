import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/restore.dart';
import 'package:wellfed/state/restore/restore.dart';
import 'package:wellfed/widgets/layout.dart';

import '../support/fakes.dart';
import 'restore_support.dart';

void main() {
  testWidgets('the facts and warnings hold the button until the owner agrees', (tester) async {
    final ports = FakeRestorePorts();
    await restoreOpen(tester, ports);

    expect(find.text('28 Oct, on this phone'), findsOneWidget);
    expect(find.text('Vendor 1.1.15'), findsOneWidget);
    expect(find.text('Matches the original'), findsOneWidget);
    expect(find.textContaining("don't carry over"), findsOneWidget);
    expect(find.textContaining("can't talk to Kitchen"), findsOneWidget);

    await tester.tap(find.widgetWithText(FilledButton, 'Restore original firmware'));
    await settle(tester);
    expect(ports.runs, isEmpty, reason: 'nothing goes before the checkbox');

    final FakeRestoreRun run = await restoreStarted(tester, ports);
    expect(ports.specs.single.backup, '/kept/stock-1.1.15.bin');
    expect(ports.specs.single.address, '10.0.0.5:7700');
    expect(ports.specs.single.dataDir, '/data');
    expect(ports.held, isNotEmpty);
    run.stopped(RestoreStop.cancelled);
    await settle(tester);
  });

  testWidgets('the steps follow the run: press, percent, restart, done', (tester) async {
    final ports = FakeRestorePorts();
    final RestoreFlow flow = await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);

    run.event(const InstallEvent.pressDispense());
    await settle(tester);
    expect(find.textContaining('Press Dispense on the feeder, once.', findRichText: true), findsOneWidget);
    expect(flow.state.committed, isFalse, reason: 'leaving is still a clean stop');

    run.event(const InstallEvent.promoting());
    run.event(const InstallEvent.written(written: 31, length: 50));
    await settle(tester);
    expect(find.textContaining('Press Dispense on the feeder, once.', findRichText: true), findsNothing);
    expect(find.text('62%'), findsOneWidget);
    expect(flow.state.committed, isTrue, reason: 'no way back once an offer may be out');

    run.event(const InstallEvent.activated());
    run.end(const RestoreEnd.activated());
    await settle(tester);
    expect(find.text('The original firmware is going back'), findsOneWidget);
    expect(find.textContaining('Hold Reset for about 5 seconds'), findsOneWidget);
    expect(ports.released, 1);
  });

  testWidgets('an uncertain end offers no retry', (tester) async {
    final ports = FakeRestorePorts();
    final RestoreFlow flow = await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);

    run.event(const InstallEvent.promoting());
    run.end(const RestoreEnd.uncertain(why: UncertainWhy.gone, detail: 'gone'));
    await settle(tester);

    expect(find.text("We can't tell if it went through"), findsOneWidget);
    expect(find.textContaining('stopped answering'), findsWidgets);
    expect(find.text('Try again'), findsNothing);
    expect(ports.runs, hasLength(1));
    expect(flow.state.mayRetry, isFalse);
  });

  testWidgets('a refusal offers no retry; a missed press asks for the word again', (tester) async {
    final ports = FakeRestorePorts();
    await restoreOpen(tester, ports);
    FakeRestoreRun run = await restoreStarted(tester, ports);
    run.stopped(RestoreStop.refused);
    await settle(tester);
    expect(find.text('Kitchen said no'), findsOneWidget);
    expect(find.text('Try again'), findsNothing);

    final again = FakeRestorePorts();
    await restoreOpen(tester, again);
    run = await restoreStarted(tester, again);
    run.stopped(RestoreStop.noPress);
    await settle(tester);
    expect(find.text('Dispense was not pressed in time'), findsOneWidget);

    await tester.tap(find.text('Try again'));
    await settle(tester);
    expect(find.text('I understand'), findsOneWidget);
    await tester.tap(find.widgetWithText(FilledButton, 'Restore original firmware'));
    await settle(tester);
    expect(again.runs, hasLength(1), reason: 'not without the checkbox again');
    final FakeRestoreRun second = await restoreStarted(tester, again);
    expect(again.runs, hasLength(2));
    second.stopped(RestoreStop.cancelled);
    await settle(tester);
  });

  testWidgets('a stream that closes with no end is uncertain once offering', (tester) async {
    final ports = FakeRestorePorts();
    final RestoreFlow flow = await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);
    run.event(const InstallEvent.promoting());
    await settle(tester);

    await run.updates.close();
    await settle(tester);

    expect(flow.state, isA<RestoreUncertain>());
  });

  testWidgets('the run asks only where the feeder is', (tester) async {
    final ports = FakeRestorePorts()..located = '10.0.0.9:7700';
    await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);

    run.updates.add(RestoreUpdate.ask(id: 3, ask: InstallAsk.locate(identity: 'ab' * 32)));
    await settle(tester);
    expect(run.answers[3], const InstallAnswer.located(address: '10.0.0.9:7700'));

    run.updates.add(const RestoreUpdate.ask(id: 4, ask: InstallAsk.closeLink()));
    await settle(tester);
    expect(run.answers[4], isA<InstallAnswer_Failed>());
    run.stopped(RestoreStop.cancelled);
    await settle(tester);
  });

  testWidgets('no kept backup says so, and a picked file takes the same checks', (tester) async {
    final ports = FakeRestorePorts()
      ..kept = null
      ..checks['/picked/good.bin'] = const BackupCheck.ready(version: '1.1.15')
      ..checks['/picked/bad.bin'] = const BackupCheck.notStock(detail: 'no');
    final RestoreFlow flow = await restoreOpen(tester, ports);
    expect(find.text('No backup on this phone'), findsOneWidget);

    // Nothing picked changes nothing.
    await tester.tap(find.text('Pick the file'));
    await settle(tester);
    expect(find.text('No backup on this phone'), findsOneWidget);

    ports.picks = '/picked/bad.bin';
    await tester.tap(find.text('Pick the file'));
    await settle(tester);
    expect(find.textContaining("That isn't the original firmware.", findRichText: true), findsOneWidget);
    expect(flow.state, isA<RestoreNoBackup>());

    ports.picks = '/picked/good.bin';
    await tester.tap(find.text('Pick the file'));
    await settle(tester);
    expect(find.text('From a picked file'), findsOneWidget);

    final FakeRestoreRun run = await restoreStarted(tester, ports);
    expect(ports.specs.single.backup, '/picked/good.bin');
    run.stopped(RestoreStop.cancelled);
    await settle(tester);
  });

  testWidgets('leaving stops the run, and lets the process go', (tester) async {
    final ports = FakeRestorePorts();
    final RestoreFlow flow = await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);

    await tester.pumpWidget(const SizedBox.shrink());
    flow.dispose();
    expect(run.stops, 1);
    expect(ports.forgotten, 0, reason: 'the run may still be using it');

    run.stopped(RestoreStop.cancelled);
    await tester.pump();
    expect(ports.released, 1);
    expect(ports.forgotten, 1);
  });

  testWidgets('a stop after the offer words itself as a stop', (tester) async {
    final ports = FakeRestorePorts();
    final RestoreFlow flow = await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);

    run.event(const InstallEvent.promoting());
    run.end(const RestoreEnd.uncertain(why: UncertainWhy.stopped, detail: 'stopped'));
    await settle(tester);

    expect(flow.state, isA<RestoreUncertain>());
    expect(find.textContaining('This was stopped'), findsOneWidget);
    expect(find.textContaining('stopped answering'), findsNothing);
  });

  testWidgets('a key this phone could not keep does not blame the backup', (tester) async {
    final ports = FakeRestorePorts();
    await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);

    run.stopped(RestoreStop.keyFailed);
    await settle(tester);

    expect(find.text("This phone couldn't set up the restore"), findsOneWidget);
    expect(find.text("That isn't the original firmware"), findsNothing);
  });

  testWidgets('a key gone before the offer has its own words and a retry', (tester) async {
    final ports = FakeRestorePorts();
    await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);

    run.stopped(RestoreStop.keyGone);
    await settle(tester);

    expect(find.text('The restore lost its key'), findsOneWidget);
    expect(find.textContaining('space or storage'), findsNothing);
    expect(find.text('Try again'), findsOneWidget);
  });

  testWidgets('a long wait says what it waits for, but not during the press', (tester) async {
    final ports = FakeRestorePorts();
    await restoreOpen(tester, ports);
    final FakeRestoreRun run = await restoreStarted(tester, ports);
    await tester.pump(WaitWords.after);
    await tester.pump();
    Finder words() => find.textContaining('answer on your Wi-Fi', findRichText: true);
    expect(words(), findsOneWidget);

    run.event(const InstallEvent.pressDispense());
    await tester.pump();
    await tester.pump(WaitWords.after);
    await tester.pump();
    expect(words(), findsNothing);
  });
}
