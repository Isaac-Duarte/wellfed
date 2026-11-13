import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';

const _kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');

PendingWrite _pending(String key, {int sentMs = 1000}) =>
    PendingWrite(key: key, feeder: 'aa', kind: WriteKind.feed, portions: 1, sentMs: sentMs, acknowledged: false);

/// The kitchen's fleet, its session open and its card read.
Future<(FeederFleet, FakeFeederPorts)> _open() async {
  final ports = FakeFeederPorts();
  final fleet = FeederFleet(FakeBook()..feeders = [_kitchen], ports);
  addTearDown(fleet.dispose);
  await pumpEventQueue();
  ports.session
    ..opened()
    ..status(fakeCard());
  await pumpEventQueue();
  return (fleet, ports);
}

void main() {
  test('every session is attached to the router before it runs', () async {
    final (_, FakeFeederPorts ports) = await _open();
    expect(ports.fakeRouter.log, ['attach aa', 'run aa']);
  });

  test("the router's answer brings a feed's key, and the session closing leaves it adrift", () async {
    final (FeederFleet fleet, FakeFeederPorts ports) = await _open();
    final FeederController kitchen = fleet.all.single;
    ports.fakeRouter.feedAnswer = const RoutedFeed.started(key: 'k1', feed: 9, portions: 1, dueSeconds: 49);

    kitchen.feedNow(1);
    await pumpEventQueue();
    expect(kitchen.state.feed, isA<FeedRunning>().having((feed) => feed.key, 'key', 'k1'));

    await ports.session.updates.close();
    await pumpEventQueue();
    expect(kitchen.state.feed, isA<FeedAdrift>().having((feed) => feed.by, 'by', FeedLoss.link));
    expect(ports.fakeRouter.fed, [('aa', 1)]);
  });

  test('an uncertain feed is checked only by reading, and putting it away lets the next one go', () async {
    final (FeederFleet fleet, FakeFeederPorts ports) = await _open();
    final FeederController kitchen = fleet.all.single;
    ports.fakeRouter.feedAnswer = RoutedFeed.uncertain(pending: _pending('k2'));
    kitchen.feedNow(1);
    await pumpEventQueue();
    expect(kitchen.state.feed, isA<FeedUnanswered>().having((feed) => feed.key, 'key', 'k2'));

    ports.fakeRouter.retryAnswer = RetryOutcome.uncertain(pending: _pending('k2'));
    kitchen.checkFeed();
    expect(kitchen.state.feed, isA<FeedUnanswered>(), reason: 'still not known');

    kitchen.dismissFeed();
    expect(ports.fakeRouter.acknowledged, ['k2']);
    expect(ports.fakeRouter.retried, ['k2']);
    expect(ports.fakeRouter.fed, [('aa', 1)], reason: 'a check never sends');
  });

  test('a check that finds the feed ended shows its end', () async {
    final (FeederFleet fleet, FakeFeederPorts ports) = await _open();
    final FeederController kitchen = fleet.all.single;
    ports.fakeRouter.feedAnswer = RoutedFeed.uncertain(pending: _pending('k3'));
    kitchen.feedNow(1);
    await pumpEventQueue();

    ports.fakeRouter.retryAnswer = RetryOutcome.settled(
      pending: _pending('k3'),
      settlement: const Settlement.fed(
        feed: 4,
        requested: 1,
        completed: 1,
        outcome: FeedOutcome.done,
        reason: FeedReason.done,
      ),
    );
    kitchen.checkFeed();
    expect(kitchen.state.feed, isA<FeedEnded>().having((feed) => feed.result.feed, 'feed', 4));
  });

  test('a feed an earlier run left uncertain is asked about before any new one', () async {
    final ports = FakeFeederPorts();
    ports.fakeRouter.held.add(_pending('k4', sentMs: 5000));
    final kitchen = FeederController(FakeBook()..feeders = [_kitchen], ports, _kitchen);
    addTearDown(kitchen.dispose);

    expect(
      kitchen.state.feed,
      isA<FeedUnanswered>()
          .having((feed) => feed.key, 'key', 'k4')
          .having((feed) => feed.sentAt, 'sent', DateTime.fromMillisecondsSinceEpoch(5000)),
    );
  });

  test('a feed held back by an uncertain one asks about that one', () async {
    final (FeederFleet fleet, FakeFeederPorts ports) = await _open();
    final FeederController kitchen = fleet.all.single;
    ports.fakeRouter.feedAnswer = RoutedFeed.held(pending: _pending('k5', sentMs: 7000));
    kitchen.feedNow(1);
    await pumpEventQueue();
    expect(
      kitchen.state.feed,
      isA<FeedUnanswered>()
          .having((feed) => feed.key, 'key', 'k5')
          .having((feed) => feed.sentAt, 'sent', DateTime.fromMillisecondsSinceEpoch(7000)),
    );
  });

  test('a feed held back by a set-aside record says to check the bowls', () async {
    final (FeederFleet fleet, FakeFeederPorts ports) = await _open();
    final FeederController kitchen = fleet.all.single;
    ports.fakeRouter.feedAnswer = const RoutedFeed.notSent(reason: NotSentReason.setAside);
    kitchen.feedNow(1);
    await pumpEventQueue();
    expect(kitchen.state.feed, isA<FeedNotSent>().having((feed) => feed.why, 'why', NotSentWhy.setAside));
  });

  test('with no router nothing is sent, not even a read', () async {
    final ports = FakeFeederPorts()..hasRouter = false;
    final kitchen = FeederController(FakeBook()..feeders = [_kitchen], ports, _kitchen);
    addTearDown(kitchen.dispose);
    await pumpEventQueue();
    ports.session.opened();
    await pumpEventQueue();

    kitchen.feedNow(1);
    expect(kitchen.state.feed, isA<FeedNotSent>().having((feed) => feed.why, 'why', NotSentWhy.notPinned));
    expect(await kitchen.ask(const FeederRequest.getPlans()), const FeederReply.notSent(reason: NotSentReason.store));
    expect(ports.session.feeds, isEmpty);
    expect(ports.session.requests, isEmpty);
  });

  test("an earlier, put-away feed's settlement never takes over a new feed still sending", () async {
    final (FeederFleet fleet, FakeFeederPorts ports) = await _open();
    final FeederController kitchen = fleet.all.single;

    // Feed B goes; its answer has not come.
    kitchen.feedNow(2);
    await pumpEventQueue();
    expect(kitchen.state.feed, isA<FeedSending>());

    // Feed A, uncertain and already put away, settles now from the history.
    final PendingWrite a = _pending('a');
    ports.fakeRouter.said.add(
      RouterUpdate(
        pending: PendingWrite(
          key: a.key,
          feeder: a.feeder,
          kind: a.kind,
          portions: a.portions,
          sentMs: a.sentMs,
          acknowledged: true,
        ),
        settlement: const Settlement.fed(
          feed: 3,
          requested: 1,
          completed: 1,
          outcome: FeedOutcome.done,
          reason: FeedReason.done,
        ),
      ),
    );
    await pumpEventQueue();
    expect(kitchen.state.feed, isA<FeedSending>(), reason: 'A says nothing of B');

    // Then B's own answer: it started.
    ports.session.updates.add(const SessionUpdate.feedStarted(feed: 7, portions: 2, dueSeconds: 49));
    await pumpEventQueue();
    expect(kitchen.state.feed, isA<FeedRunning>(), reason: 'B runs');
  });

  testWidgets('a feed the router later says started shows as uncertain once it is past due', (tester) async {
    final ports = FakeFeederPorts();
    final fleet = FeederFleet(FakeBook()..feeders = [_kitchen], ports);
    addTearDown(fleet.dispose);
    await tester.pump();
    ports.session
      ..opened()
      ..status(fakeCard());
    await tester.pump();
    final FeederController kitchen = fleet.all.single;
    ports.fakeRouter.feedAnswer = RoutedFeed.uncertain(pending: _pending('k6'));
    kitchen.feedNow(1);
    await tester.pump();

    // A screen made again while the call was out holds only the pin, and
    // hears the call's answer by its key.
    ports.fakeRouter.said.add(RouterUpdate(pending: _pending('k6'), settlement: const Settlement.started(feed: 5)));
    await tester.pump();
    expect(kitchen.state.feed, isA<FeedRunning>().having((feed) => feed.feed, 'feed', 5));

    await tester.pump(const Duration(seconds: FakeFeederPorts.feedDue + 1));
    expect(kitchen.state.feed, isA<FeedAdrift>().having((feed) => feed.by, 'by', FeedLoss.overdue));
    expect(kitchen.state.feed.active, isFalse, reason: 'it no longer blocks the next feed');
  });
}
