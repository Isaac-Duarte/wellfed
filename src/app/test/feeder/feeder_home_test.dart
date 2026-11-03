import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/app.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/home/home.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/theme/theme.dart';

import '../support/fakes.dart';
import 'feeder_support.dart';

void main() {
  group('Home', () {
    Future<(FeederFleet, FakeSession)> home(
      WidgetTester tester, {
      PendingInstall earlier = PendingInstall.nothing,
    }) async {
      feederPhone(tester);
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      final GoRouter router = wellfedRouter(
        book: FakeBook(),
        root: (context) => HomeScreen(fleet: fleet, earlierInstall: () async => earlier),
        pages: {
          '/settings': (context) {
            final book = FakeBook();
            return AppSettingsScreen(book: book, newBackups: () => BackupsFlow(book, FakeBackupPorts()));
          },
        },
        feeder: (context, identity, tab) => FeederRoute(fleet: fleet, identity: identity, initialTab: tab),
      );
      addTearDown(router.dispose);
      await tester.pumpWidget(MaterialApp.router(theme: wellfedTheme(Brightness.light), routerConfig: router));
      await settle(tester);
      return (fleet, session);
    }

    testWidgets('the gear opens the app settings, and back comes home', (tester) async {
      await home(tester);
      expect(find.byType(NavigationBar), findsNothing);

      await feederTap(tester, find.byTooltip('Settings'));
      expect(find.text('Appearance'.toUpperCase()), findsOneWidget);

      await feederTap(tester, find.byTooltip('Back'));
      expect(find.text('Feeders'), findsOneWidget);
      expect(find.byTooltip('Settings'), findsOneWidget);
    });

    testWidgets('says when an earlier install did not finish', (tester) async {
      await home(tester);
      expect(find.text('Finish installing'), findsNothing);

      await home(tester, earlier: PendingInstall.values.last);
      expect(find.text('Finish installing'), findsOneWidget);
    });

    testWidgets('a last feed with no time says so', (tester) async {
      final (_, FakeSession session) = await home(tester);
      session.status(
        fakeCard(
          lastFed: fakeFed(confirmed: 0, atMost: 2, outcome: FeedOutcome.uncertain, reason: FeedReason.interrupted),
        ),
      );
      await settle(tester);

      expect(find.text('Time unknown'), findsOneWidget);
      expect(find.text('Unknown'), findsNothing);
      expect(find.text('0 to 2 of 2'), findsOneWidget);
    });

    testWidgets("shows the next meal, the last feed and today's meals", (tester) async {
      final (_, FakeSession session) = await home(tester);
      final now = DateTime.now();
      session.status(
        fakeCard(
          nextMeal: NextMealView(
            slot: 1,
            atMs: now.add(const Duration(minutes: 26)).millisecondsSinceEpoch,
            minute: 12 * 60 + 30,
            portions: 2,
          ),
          lastFed: fakeFed(atMs: feederTodayAt(7, 30)),
          meals: [
            MealView(
              slot: 0,
              minute: 7 * 60 + 30,
              portions: 2,
              call: 0,
              standing: MealStanding.fed(result: fakeFed()),
            ),
            MealView(
              slot: 1,
              minute: 12 * 60 + 30,
              portions: 2,
              call: 0,
              standing: MealStanding.next(
                atMs: now.add(const Duration(minutes: 26, seconds: 30)).millisecondsSinceEpoch,
              ),
            ),
            const MealView(slot: 2, minute: 18 * 60 + 30, portions: 3, call: 1, standing: MealStanding.later()),
          ],
        ),
      );
      await settle(tester);

      expect(find.text('Next meal'), findsOneWidget);
      expect(find.text('Last fed'), findsOneWidget);
      expect(find.text('2 of 2'), findsOneWidget);
      expect(find.text("Today's meals"), findsOneWidget);
      expect(find.text('Fed'), findsOneWidget);
      expect(find.text('in 26 min'), findsOneWidget);
      expect(find.text('3 portions · meal call'), findsOneWidget);
      expect(find.text('Later'), findsOneWidget);
    });

    testWidgets('the card opens the feeder, and Schedule its tab', (tester) async {
      final (_, FakeSession session) = await home(tester);
      session.status(
        fakeCard(
          meals: const [MealView(slot: 2, minute: 18 * 60 + 30, portions: 3, call: 0, standing: MealStanding.later())],
        ),
      );
      await settle(tester);

      await feederTap(tester, find.widgetWithText(TextButton, 'Schedule'));
      final TabController tabs = DefaultTabController.of(tester.element(find.byType(TabBar)));
      expect(tabs.index, 1);
    });

    testWidgets("a feeder with no clock offers this phone's", (tester) async {
      final (_, FakeSession session) = await home(tester);
      session.status(fakeCard(clockSet: false));
      await settle(tester);

      expect(find.textContaining('Meals are on hold.', findRichText: true), findsOneWidget);
      await feederTap(tester, find.text('Set the time from this phone'));
      expect(session.requests, [const FeederRequest.syncClock()]);
    });
  });

  group('Feed now', () {
    Future<FakeSession> sheet(WidgetTester tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fakeCard());
      await settle(tester);
      await feederTap(tester, find.widgetWithText(FilledButton, 'Feed now'));
      return session;
    }

    testWidgets('speaks in grams once the portion size is known', (tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      session.answering = (request) async => switch (request) {
        FeederRequest_GetCalibration() => const FeederReply.calibration(
          calibration: CalibrationInfo(generation: 1, writable: true, portionMg: 12000, bowl: Bowl.single),
        ),
        _ => const FeederReply.done(),
      };
      await feederShowShell(tester, fleet);
      session.status(fakeCard());
      await settle(tester);
      await feederTap(tester, find.widgetWithText(FilledButton, 'Feed now'));

      expect(session.requests, contains(const FeederRequest.getCalibration()));
      expect(find.text('≈ 12 g'), findsOneWidget);
      expect(find.textContaining('Each portion is about 12 g.'), findsOneWidget);
      await feederTap(tester, find.byTooltip('One more portion'));
      expect(find.text('≈ 24 g'), findsOneWidget);
      await feederTap(tester, find.text('Feed 2 portions · 24 g'));
      expect(session.feeds, [2]);
    });

    testWidgets('goes out once, and a refusal offers a new start', (tester) async {
      final FakeSession session = await sheet(tester);
      expect(find.textContaining("Today's meals still run."), findsOneWidget);
      expect(find.textContaining("one turn of the feeder's wheel"), findsOneWidget);
      expect(find.textContaining('≈'), findsNothing);

      await feederTap(tester, find.text('Feed 1 portion'));
      expect(session.feeds, [1]);

      session.updates.add(const SessionUpdate.feedRefused(refusal: FeedRefusal.busy));
      await settle(tester);
      expect(find.text("Kitchen didn't feed"), findsOneWidget);
      expect(find.text('Start over'), findsOneWidget);
    });

    testWidgets('an unanswered feed is never offered again', (tester) async {
      final FakeSession session = await sheet(tester);
      await feederTap(tester, find.text('Feed 1 portion'));

      session.updates.add(const SessionUpdate.feedUnanswered());
      await settle(tester);
      expect(find.text('Did Kitchen feed?'), findsOneWidget);
      expect(find.text('Start over'), findsNothing);
      expect(session.feeds, [1]);
    });

    testWidgets('Feed now after a known result, tapped away, asks portions', (tester) async {
      final FakeSession session = await sheet(tester);
      await feederTap(tester, find.text('Feed 1 portion'));
      session.updates.add(const SessionUpdate.feedStarted(feed: 7, portions: 2, dueSeconds: 49));
      await settle(tester);
      session.updates.add(SessionUpdate.feedEnded(result: fakeFed()));
      await settle(tester);
      expect(find.text('Fed 2 of 2 portions'), findsOneWidget);

      await tester.tapAt(const Offset(10, 10));
      await settle(tester);
      expect(find.text('Fed 2 of 2 portions'), findsNothing);

      await feederTap(tester, find.text('Feed now'));
      expect(find.text('Fed 2 of 2 portions'), findsNothing);
      expect(find.text('Feed 1 portion'), findsOneWidget);
    });

    testWidgets('Feed now after an unknown result, tapped away, still says so', (tester) async {
      final FakeSession session = await sheet(tester);
      await feederTap(tester, find.text('Feed 1 portion'));
      session.updates.add(const SessionUpdate.feedUnanswered());
      await settle(tester);
      expect(find.text('Did Kitchen feed?'), findsOneWidget);

      await tester.tapAt(const Offset(10, 10));
      await settle(tester);
      await feederTap(tester, find.text('Feed now'));

      expect(find.text('Did Kitchen feed?'), findsOneWidget);
      expect(find.text('Feed 1 portion'), findsNothing);
      expect(session.feeds, [1]);
    });

    testWidgets('an uncertain feed shows a range, and the history', (tester) async {
      final FakeSession session = await sheet(tester);
      await feederTap(tester, find.text('Feed 1 portion'));
      session.updates.add(const SessionUpdate.feedStarted(feed: 7, portions: 2, dueSeconds: 49));
      await settle(tester);
      session.updates.add(
        SessionUpdate.feedEnded(
          result: fakeFed(confirmed: 0, atMost: 2, outcome: FeedOutcome.uncertain, reason: FeedReason.power),
        ),
      );
      await settle(tester);

      expect(find.text('Fed between 0 and 2 portions'), findsOneWidget);
      expect(find.text('See history'), findsOneWidget);
      expect(find.text('Start over'), findsNothing);
      expect(session.feeds, [1]);
    });

    group('when the feeder goes away', () {
      /// The sheet following a running feed of 2 portions, numbered 7.
      Future<FakeFeederPorts> running(WidgetTester tester) async {
        final book = FakeBook()..feeders = [feederKitchen];
        final ports = FakeFeederPorts();
        final fleet = FeederFleet(book, ports);
        addTearDown(fleet.dispose);
        await tester.pump();
        ports.session.opened();
        await tester.pump();
        await feederShowShell(tester, fleet);
        ports.session.status(fakeCard());
        await settle(tester);
        await feederTap(tester, find.widgetWithText(FilledButton, 'Feed now'));
        await feederTap(tester, find.byTooltip('One more portion'));
        await feederTap(tester, find.text('Feed 2 portions'));
        ports.session.updates.add(const SessionUpdate.feedStarted(feed: 7, portions: 2, dueSeconds: 60));
        // One pump delivers it, so the due time counts from here exactly.
        await tester.pump();
        expect(find.text('Feeding Kitchen'), findsOneWidget);
        return ports;
      }

      const didFeed = 'Did Kitchen feed?';
      const justBefore = Duration(seconds: 59);

      testWidgets('a healthy slow feed reaches its result without going lost', (tester) async {
        final FakeFeederPorts ports = await running(tester);
        await tester.pump(justBefore);
        expect(find.text('Feeding Kitchen'), findsOneWidget);

        ports.session.updates.add(SessionUpdate.feedEnded(result: fakeFed()));
        await settle(tester);
        expect(find.text('Fed 2 of 2 portions'), findsOneWidget);
      });

      testWidgets('an overdue feed goes lost, and its late result replaces it', (tester) async {
        final FakeFeederPorts ports = await running(tester);
        await tester.pump(justBefore + const Duration(seconds: 2));
        await settle(tester);
        expect(find.text(didFeed), findsOneWidget);
        expect(find.text('Stop feeding'), findsNothing);
        expect(find.textContaining("hasn't said how it ended, past the longest a feed can take."), findsOneWidget);
        expect(find.textContaining('connection dropped'), findsNothing);
        expect(
          find.text(
            "Wellfed won't send it again on its own. Look at the bowl before "
            'you feed again.',
          ),
          findsOneWidget,
        );

        // Another feed's end says nothing about this one.
        ports.session.updates.add(SessionUpdate.feedEnded(result: fakeFed(feed: 8)));
        await settle(tester);
        expect(find.text(didFeed), findsOneWidget);

        ports.session.updates.add(SessionUpdate.feedEnded(result: fakeFed()));
        await settle(tester);
        expect(find.text('Fed 2 of 2 portions'), findsOneWidget);
      });

      testWidgets('one feed goes out across going lost and reconnecting', (tester) async {
        final FakeFeederPorts ports = await running(tester);
        final FakeSession first = ports.session;

        // The session ends on the link's silence.
        first.updates.add(
          const SessionUpdate.closed(
            error: BridgeError(kind: ErrorKind.closed, message: 'silent'),
          ),
        );
        await first.updates.close();
        await settle(tester);
        expect(find.text(didFeed), findsOneWidget);
        expect(
          find.textContaining(
            'then the connection dropped. If the feeder stayed on, it '
            'finishes on its own and keeps the result in its history.',
          ),
          findsOneWidget,
        );

        // Long enough for the controller to try again.
        await tester.pump(const Duration(seconds: 10));
        expect(ports.sessions.length, greaterThan(1));
        ports.session.opened();
        await settle(tester);
        ports.session.status(fakeCard());
        await settle(tester);

        // Still lost, and nothing asks the new session for a feed: only the
        // owner, from the start of the sheet, can.
        await tester.pump(const Duration(minutes: 5));
        expect(find.text(didFeed), findsOneWidget);
        expect(find.text('Start over'), findsNothing);
        expect(ports.sessions.expand((session) => session.feeds), [2]);

        // The restarted feeder's record of it settles what the bowl was to say.
        ports.session.history([
          HistoryEntry(
            seq: BigInt.from(9),
            body: HistoryBody.fed(
              result: fakeFed(confirmed: 0, atMost: 2, outcome: FeedOutcome.uncertain, reason: FeedReason.power),
            ),
          ),
        ]);
        await settle(tester);
        expect(find.text('Fed between 0 and 2 portions'), findsOneWidget);
        expect(ports.sessions.expand((session) => session.feeds), [2]);
      });

      testWidgets('stop never says it stopped, and says when it could not ask', (tester) async {
        final FakeFeederPorts ports = await running(tester);
        await feederTap(tester, find.text('Stop feeding'));
        expect(ports.session.cancels, [7]);
        expect(find.text('Feeding Kitchen'), findsOneWidget);
        expect(find.text('Asked the feeder to stop. Waiting for it to answer.'), findsOneWidget);

        ports.session.cancelFails = const BridgeError(kind: ErrorKind.closed, message: 'the session ended');
        await feederTap(tester, find.text('Stop feeding'));
        expect(find.text('Feeding Kitchen'), findsOneWidget);
        expect(find.textContaining('The feeder did not confirm the stop'), findsOneWidget);

        ports.session.updates.add(SessionUpdate.feedEnded(result: fakeFed()));
        await settle(tester);
      });
    });
  });

  test('a request with no session open never went out', () async {
    final book = FakeBook()..feeders = [feederKitchen];
    final ports = FakeFeederPorts();
    // Not opened yet in the first microtask: no session, nothing sent.
    final controller = FeederController(book, ports, feederKitchen);
    final FeederReply reply = await controller.ask(const FeederRequest.reboot());
    expect(reply, const FeederReply.notSent());
    controller.dispose();
  });
}
