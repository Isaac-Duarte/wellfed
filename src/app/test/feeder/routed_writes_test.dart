import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/home/home.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart' show PendingInstall;
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/widgets/replies.dart';

import '../support/fakes.dart';

const _kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');

PendingWrite _held(String key, WriteKind kind) => PendingWrite(
  key: key,
  feeder: 'aa',
  kind: kind,
  portions: kind == WriteKind.feed ? 1 : null,
  sentMs: 1000,
  acknowledged: false,
);

void main() {
  test('a feed the router took up while its first screen was gone settles by its key on the next', () async {
    final ports = FakeFeederPorts();
    final first = FeederController(FakeBook()..feeders = [_kitchen], ports, _kitchen);
    // The fleet made the controller again with the feed still out: the new
    // one takes its pin up from the router, as uncertain.
    ports.fakeRouter.held.add(_held('k7', WriteKind.feed));
    first.dispose();
    final again = FeederController(FakeBook()..feeders = [_kitchen], ports, _kitchen);
    addTearDown(again.dispose);
    expect(again.state.feed, isA<FeedUnanswered>().having((feed) => feed.key, 'key', 'k7'));

    // The call's answer went to the first; the router tells it by key.
    again.routerSaid(
      RouterUpdate(pending: _held('k7', WriteKind.feed), settlement: const Settlement.started(feed: 12)),
    );
    expect(again.state.feed, isA<FeedRunning>().having((feed) => feed.feed, 'feed', 12));

    // Told twice (the call's answer, then the update): still the one feed.
    final FeedState running = again.state.feed;
    again.routerSaid(
      RouterUpdate(pending: _held('k7', WriteKind.feed), settlement: const Settlement.started(feed: 12)),
    );
    expect(again.state.feed, same(running));
  });

  test('a request the bridge fails reads as unanswered, never as a throw', () async {
    final ports = FakeFeederPorts();
    ports.fakeRouter.routed = (request) => throw StateError('a bridge panic');
    final feeder = FeederController(FakeBook()..feeders = [_kitchen], ports, _kitchen);
    addTearDown(feeder.dispose);

    final FeederReply reply = await feeder.ask(const FeederRequest.setLock(locked: true));
    expect(reply, const FeederReply.unanswered());
  });

  test('a change that never said how it ended is shown until it is put away', () async {
    final ports = FakeFeederPorts();
    ports.fakeRouter.held.addAll([_held('s1', WriteKind.settings), _held('f1', WriteKind.feed)]);
    final kitchen = FeederController(FakeBook()..feeders = [_kitchen], ports, _kitchen);
    addTearDown(kitchen.dispose);

    expect(kitchen.state.unconfirmed.map((pending) => pending.key), ['s1'], reason: 'a feed is the sheet’s');

    // The router let it go once the owner looked.
    ports.fakeRouter.held.removeWhere((pending) => pending.key == 's1');
    kitchen.putAwayWrite('s1');
    expect(ports.fakeRouter.acknowledged, ['s1']);
    expect(kitchen.state.unconfirmed, isEmpty);
  });

  testWidgets('a change left unanswered is pinned, shown at once, and never sent again', (tester) async {
    final ports = FakeFeederPorts();
    final kitchen = FeederController(FakeBook()..feeders = [_kitchen], ports, _kitchen);
    addTearDown(kitchen.dispose);
    await tester.pump();
    ports.session
      ..opened()
      ..status(fakeCard());
    await tester.pump();
    ports.session.answering = (request) async => const FeederReply.unanswered();
    await tester.pumpWidget(themed(FeederShell(feeder: kitchen)));
    await settle(tester);

    final FeederReply reply = await kitchen.ask(const FeederRequest.setLock(locked: true));
    await settle(tester);
    expect(replyProblem(reply), contains('may or may not have gone through'));
    expect(kitchen.state.unconfirmed.map((pending) => pending.kind), [WriteKind.lock]);
    expect(find.textContaining('may not have reached the feeder', findRichText: true), findsOneWidget);
    expect(ports.session.requests, hasLength(1), reason: 'nothing sends it again');
  });

  test('a change that never went says why, and what to do next', () async {
    final ports = FakeFeederPorts();
    final kitchen = FeederController(FakeBook()..feeders = [_kitchen], ports, _kitchen);
    addTearDown(kitchen.dispose);
    RoutedReply notSent(NotSentReason reason) => RoutedReply(reply: FeederReply.notSent(reason: reason));

    for (final (routed, words) in [
      (notSent(NotSentReason.noPath), "can't reach the feeder"),
      (notSent(NotSentReason.store), "couldn't note it down"),
      (notSent(NotSentReason.session), 'The connection dropped'),
    ]) {
      ports.fakeRouter.routed = (_) => routed;
      final String? problem = replyProblem(await kitchen.ask(const FeederRequest.resetDesiccant()));
      expect(problem, contains(words), reason: '$routed');
      expect(problem, contains('Nothing changed'));
    }
    expect(kitchen.state.unconfirmed, isEmpty);
  });

  testWidgets('the banner names the change and offers only putting it away', (tester) async {
    final ports = FakeFeederPorts();
    ports.fakeRouter.held.add(_held('s1', WriteKind.plans));
    final kitchen = FeederController(FakeBook()..feeders = [_kitchen], ports, _kitchen);
    addTearDown(kitchen.dispose);
    await tester.pumpWidget(themed(FeederShell(feeder: kitchen)));
    await settle(tester);

    expect(
      find.textContaining('A schedule change may not have reached the feeder.', findRichText: true),
      findsOneWidget,
    );
    expect(find.text('Check'), findsNothing, reason: 'no history names a change other than a feed');
    await tester.tap(find.text('Put away'));
    await settle(tester);
    expect(ports.fakeRouter.acknowledged, ['s1']);
    expect(ports.session.requests, isEmpty, reason: 'nothing is sent again');
  });

  testWidgets('a record of unconfirmed writes that will not open is said plainly on Home', (tester) async {
    tester.view.physicalSize = const Size(412, 915);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final fleet = FeederFleet(FakeBook()..feeders = [_kitchen], FakeFeederPorts()..hasRouter = false);
    addTearDown(fleet.dispose);
    await tester.pumpWidget(
      themed(
        HomeScreen(
          fleet: fleet,
          earlierInstall: () async => PendingInstall.nothing,
          pinsProblem: PinsProblem.unavailable,
        ),
      ),
    );
    await settle(tester);

    expect(find.textContaining('Feeding and changes are off.', findRichText: true), findsOneWidget);
    expect(find.textContaining('Close the app and open it again', findRichText: true), findsOneWidget);
    expect(find.textContaining("Clear the app's data", findRichText: true), findsNothing);
  });
}
