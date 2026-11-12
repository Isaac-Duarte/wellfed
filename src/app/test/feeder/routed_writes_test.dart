import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/feeder/today.dart';
import 'package:wellfed/screens/home/home.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart' show PendingInstall;
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/server/server.dart';
import 'package:wellfed/widgets/replies.dart';

import '../support/fakes.dart';

const _kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');

PendingWrite _held(String key, WriteKind kind) => PendingWrite(
  key: key,
  feeder: 'aa',
  path: RoutePath.direct,
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
      RouterUpdate.settled(pending: _held('k7', WriteKind.feed), settlement: const Settlement.started(feed: 12)),
    );
    expect(again.state.feed, isA<FeedRunning>().having((feed) => feed.feed, 'feed', 12));

    // Told twice (the call's answer, then the update): still the one feed.
    final FeedState running = again.state.feed;
    again.routerSaid(
      RouterUpdate.settled(pending: _held('k7', WriteKind.feed), settlement: const Settlement.started(feed: 12)),
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

  test('a change that never said how it ended is shown, checked only by reading, and put away', () async {
    final ports = FakeFeederPorts();
    ports.fakeRouter.held.addAll([_held('s1', WriteKind.settings), _held('f1', WriteKind.feed)]);
    final kitchen = FeederController(FakeBook()..feeders = [_kitchen], ports, _kitchen);
    addTearDown(kitchen.dispose);

    expect(kitchen.state.unconfirmed.map((pending) => pending.key), ['s1'], reason: 'a feed is the sheet’s');
    await kitchen.checkWrite('s1');
    expect(ports.fakeRouter.retried, ['s1']);
    expect(ports.session.requests, isEmpty, reason: 'a check sends nothing');

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
    RoutedReply notSent(NotSentReason reason, [ServerErrorKind? kind]) => RoutedReply(
      path: RoutePath.server,
      reply: FeederReply.notSent(
        reason: reason,
        server: kind == null ? null : ServerError(kind: kind, message: 'no', mayHaveRun: false),
      ),
    );

    for (final (routed, words) in [
      (notSent(NotSentReason.server, ServerErrorKind.tokenRevoked), 'Log in again'),
      (notSent(NotSentReason.server, ServerErrorKind.forbidden), 'may not do that'),
      (notSent(NotSentReason.server, ServerErrorKind.feederOffline), "can't reach the feeder"),
      (notSent(NotSentReason.noPath), "can't reach the feeder"),
      (notSent(NotSentReason.store), "couldn't note it down"),
      (notSent(NotSentReason.session), 'The connection dropped'),
    ]) {
      ports.fakeRouter.routed = (_) => routed;
      final String? problem = replyProblem(await kitchen.ask(const FeederRequest.resetDesiccant()));
      expect(problem, contains(words), reason: '$routed');
      expect(problem, contains('Nothing changed'));
      if (routed.reply case FeederReply_NotSent(server: ServerError())) {
        expect(problem, isNot(contains('The connection dropped')), reason: 'the server said why');
      }
    }
    expect(kitchen.state.unconfirmed, isEmpty);
  });

  testWidgets('the banner names the change and offers only a check', (tester) async {
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
    await tester.tap(find.text('Check'));
    await settle(tester);
    expect(ports.fakeRouter.retried, ['s1']);
  });

  testWidgets("a viewer's feeders have no Feed", (tester) async {
    tester.view.physicalSize = const Size(412, 915);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final book = FakeBook()
      ..feeders = [_kitchen]
      ..server = const KnownServer(
        link: serverLink,
        feeders: [ListedFeeder(id: 'id-k', identity: 'aa', name: 'Kitchen')],
        account: ServerAccount(id: 'u2', name: 'sam', role: ServerRole.viewer),
      );
    final ports = FakeFeederPorts();
    final serverPorts = FakeServerPorts()
      ..kept = 'token-secret'
      ..session = FakeServerSession(
        who: const ServerAccount(id: 'u2', name: 'sam', role: ServerRole.viewer),
      );
    final server = HomeServer(book, serverPorts, ports.router, FakeClientKey());
    final fleet = FeederFleet(book, ports, server: server);
    addTearDown(() {
      fleet.dispose();
      server.dispose();
    });
    await tester.pump();
    ports.session
      ..opened()
      ..status(fakeCard());
    await tester.pump();

    final FeederController kitchen = fleet.all.single;
    expect(kitchen.state.viewer, isTrue);
    expect(kitchen.state.canFeed, isFalse);
    await tester.pumpWidget(themed(Scaffold(body: TodayTab(feeder: kitchen))));
    await settle(tester);
    expect(find.text('Feed now'), findsNothing);
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
