import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/app.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/home.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder.dart';
import 'package:wellfed/state/fleet.dart';
import 'package:wellfed/theme/theme.dart';

import 'fakes.dart';

const _identity = 'aa';
const _kitchen = KnownFeeder(
  identity: _identity,
  name: 'Kitchen',
  address: 'lan:4747',
);

/// One known feeder, its session opened.
Future<(FeederFleet, FakeSession)> _fleet(WidgetTester tester) async {
  final book = FakeBook()..feeders = [_kitchen];
  final ports = FakeFeederPorts();
  final fleet = FeederFleet(book, ports);
  addTearDown(fleet.dispose);
  await tester.pump();
  ports.session.opened();
  await tester.pump();
  return (fleet, ports.session);
}

FeederController _only(FeederFleet fleet) => fleet.all.single;

/// `hour`:`minute` today, in milliseconds: a time with no day on it stays bare
/// only while it is today's.
int _todayAt(int hour, int minute) {
  final now = DateTime.now();
  return DateTime(
    now.year,
    now.month,
    now.day,
    hour,
    minute,
  ).millisecondsSinceEpoch;
}

/// A phone-sized frame, so the whole tab fits as the board draws it.
void _phone(WidgetTester tester) {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

Future<void> _showShell(WidgetTester tester, FeederFleet fleet) async {
  _phone(tester);
  await tester.pumpWidget(themed(FeederShell(feeder: _only(fleet))));
  await settle(tester);
}

/// Tap and let the fake's answer come back.
Future<void> _tap(WidgetTester tester, Finder target) async {
  await tester.ensureVisible(target);
  await tester.pump();
  await tester.tap(target);
  await settle(tester);
}

void main() {
  testWidgets('Today says how the feeder stands, as the feeder said it', (
    tester,
  ) async {
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);

    final fedAt = _todayAt(7, 30);
    session.status(
      fakeCard(
        nextMeal: NextMealView(
          atMs: _todayAt(12, 30),
          minute: 12 * 60 + 30,
          portions: 2,
        ),
        lastFed: fakeFed(atMs: fedAt),
        time: TimeInfo(
          generation: 1,
          writable: true,
          ntp: const NtpView.gateway(port: 123),
          syncs: 3,
          last: TimeSource.ntp,
          lastAtMs: _todayAt(11, 58),
        ),
      ),
    );
    await settle(tester);

    for (final tab in ['Today', 'Schedule', 'History', 'Settings']) {
      expect(find.text(tab), findsOneWidget);
    }
    expect(find.text('Ready to feed'), findsOneWidget);
    expect(find.text('Next meal 12:30 PM · 2 portions'), findsOneWidget);
    expect(find.text('Last fed 7:30 AM · 2 of 2'), findsOneWidget);
    expect(find.text('In the hopper at 7:30 AM'), findsOneWidget);
    expect(find.text('Mains · batteries 90%'), findsOneWidget);
    expect(find.text('Strong'), findsOneWidget);
    expect(find.text('Set by your router at 11:58 AM'), findsOneWidget);
    expect(find.text('Change in 9 days'), findsOneWidget);
    expect(find.text('Changed today'), findsNothing);
  });

  testWidgets('a moment from an earlier day says which day', (tester) async {
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);

    final yesterday = DateTime.now().subtract(const Duration(days: 1));
    final at = DateTime(
      yesterday.year,
      yesterday.month,
      yesterday.day,
      21,
    ).millisecondsSinceEpoch;
    session.status(
      fakeCard(
        lastFed: fakeFed(atMs: at),
        time: TimeInfo(
          generation: 1,
          writable: true,
          ntp: const NtpView.off(),
          syncs: 3,
          last: TimeSource.client,
          lastAtMs: at,
        ),
      ),
    );
    await settle(tester);

    expect(find.text('Last fed Yesterday 9:00 PM · 2 of 2'), findsOneWidget);
    expect(find.text('In the hopper yesterday at 9:00 PM'), findsOneWidget);
    expect(
      find.text('Set by a paired device yesterday at 9:00 PM'),
      findsOneWidget,
    );
  });

  testWidgets('each fact is its own node, and the lock its own switch', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);
    session.status(fakeCard(locked: false));
    await settle(tester);

    final food = tester.getSemantics(find.text('Food'));
    expect(food.label, contains('Food'));
    expect(food.label, isNot(contains('Wi-Fi')));
    expect(food.label, isNot(contains('Lock the buttons')));
    expect(tester.getSemantics(find.text('Wi-Fi')), isNot(same(food)));

    final lock = tester.getSemantics(find.byType(Switch));
    expect(
      lock,
      matchesSemantics(
        label: 'Lock the buttons',
        hasToggledState: true,
        hasEnabledState: true,
        isEnabled: true,
        isFocusable: true,
        hasTapAction: true,
        hasFocusAction: true,
      ),
    );
    final bounds = tester.getSize(find.byType(Switch));
    expect(lock.rect.width, lessThanOrEqualTo(bounds.width));

    semantics.dispose();
  });

  testWidgets('a low reading says low, and never that the hopper is empty', (
    tester,
  ) async {
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);

    session.status(fakeCard(food: FoodLevel.low));
    await settle(tester);

    expect(find.textContaining('Low'), findsWidgets);
    expect(find.textContaining('empty'), findsNothing);
    expect(find.textContaining('Hopper'), findsNothing);
  });

  testWidgets('a low reading at rest borrows no time from the last feed', (
    tester,
  ) async {
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);

    // The last feed read food; the level has gone low since, with no event.
    final fedAt = _todayAt(7, 30);
    session.status(
      fakeCard(
        food: FoodLevel.low,
        lastFed: fakeFed(atMs: fedAt),
      ),
    );
    await settle(tester);

    expect(find.text('Low'), findsOneWidget);
    expect(find.text('Low at 7:30 AM'), findsNothing);
    expect(find.textContaining('read low at'), findsNothing);
  });

  testWidgets('an uncertain last feed is a range, never one number', (
    tester,
  ) async {
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);

    session.status(
      fakeCard(
        lastFed: fakeFed(
          confirmed: 0,
          atMost: 2,
          outcome: FeedOutcome.uncertain,
          reason: FeedReason.interrupted,
        ),
      ),
    );
    await settle(tester);

    expect(find.text('Last fed 0 to 2 of 2'), findsOneWidget);
  });

  testWidgets('a one-portion feed that stopped says portion, not portions', (
    tester,
  ) async {
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);

    session.status(
      fakeCard(
        fault: FaultView(
          feed: 7,
          outcome: FeedOutcome.uncertain,
          reason: FeedReason.power,
          result: fakeFed(
            confirmed: 0,
            atMost: 1,
            requested: 1,
            outcome: FeedOutcome.uncertain,
            reason: FeedReason.power,
          ),
        ),
      ),
    );
    await settle(tester);

    expect(
      find.textContaining('It stopped somewhere between 0 and 1 of 1 portion.'),
      findsOneWidget,
    );
  });

  testWidgets('a feed that stopped unsure says the range in words', (
    tester,
  ) async {
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);

    session.status(
      fakeCard(
        fault: FaultView(
          feed: 7,
          outcome: FeedOutcome.uncertain,
          reason: FeedReason.power,
          result: fakeFed(
            confirmed: 0,
            atMost: 2,
            outcome: FeedOutcome.uncertain,
            reason: FeedReason.power,
          ),
        ),
      ),
    );
    await settle(tester);

    expect(
      find.textContaining(
        'It stopped somewhere between 0 and 2 of 2 portions.',
      ),
      findsOneWidget,
    );
  });

  testWidgets("History's filter survives a tab away while the screen is open", (
    tester,
  ) async {
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);
    session.status(fakeCard());
    await settle(tester);

    bool problems() => tester
        .widget<ChoiceChip>(find.widgetWithText(ChoiceChip, 'Problems'))
        .selected;

    await _tap(tester, find.text('History'));
    expect(problems(), isFalse);
    // A plain tap: scrolling it into view would slide the tabs' page as well.
    await tester.tap(find.widgetWithText(ChoiceChip, 'Problems'));
    await settle(tester);
    expect(problems(), isTrue);

    await _tap(tester, find.text('Today'));
    expect(find.widgetWithText(ChoiceChip, 'Problems'), findsNothing);
    await _tap(tester, find.text('History'));
    expect(problems(), isTrue);
  });

  group('the lock', () {
    Future<FakeSession> locked(WidgetTester tester) async {
      final (fleet, session) = await _fleet(tester);
      await _showShell(tester, fleet);
      session.status(fakeCard(locked: false));
      await settle(tester);
      return session;
    }

    testWidgets('goes out once and follows what the feeder says', (
      tester,
    ) async {
      final session = await locked(tester);

      await _tap(tester, find.byType(Switch));
      expect(session.requests, [const FeederRequest.setLock(locked: true)]);
      // Not moved by asking: only the status says it is locked.
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);

      session.status(fakeCard(locked: true));
      await settle(tester);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
    });

    testWidgets('says why it was refused', (tester) async {
      final session = await locked(tester);
      session.replies.add(
        const FeederReply.refused(refusal: FeederRefusal.forbidden()),
      );

      await _tap(tester, find.byType(Switch));
      expect(
        find.text("This phone isn't allowed to do that here."),
        findsOneWidget,
      );
    });

    testWidgets('says an unanswered change is unknown, and sends it once', (
      tester,
    ) async {
      final session = await locked(tester);
      session.replies.add(const FeederReply.unanswered());

      await _tap(tester, find.byType(Switch));
      expect(find.textContaining('may or may not have'), findsOneWidget);
      await settle(tester);
      expect(session.requests, hasLength(1));
    });
  });

  group('the desiccant', () {
    Future<FakeSession> due(WidgetTester tester) async {
      final (fleet, session) = await _fleet(tester);
      await _showShell(tester, fleet);
      session.status(
        fakeCard(desiccant: const Desiccant.remaining(days: -3, due: true)),
      );
      await settle(tester);
      return session;
    }

    testWidgets('records a change when it is due', (tester) async {
      final session = await due(tester);

      expect(find.text('3 days overdue'), findsOneWidget);
      await _tap(tester, find.text('Changed today'));
      expect(session.requests, [const FeederRequest.resetDesiccant()]);
    });

    testWidgets('says why the feeder refused it, or that it may have run', (
      tester,
    ) async {
      final session = await due(tester);
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.noClock()))
        ..add(const FeederReply.unanswered());

      await _tap(tester, find.text('Changed today'));
      expect(find.textContaining('Set its time first'), findsOneWidget);

      await _tap(tester, find.text('Changed today'));
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(session.requests, hasLength(2));
    });
  });

  group('a fault', () {
    Future<FakeSession> stopped(WidgetTester tester) async {
      final (fleet, session) = await _fleet(tester);
      await _showShell(tester, fleet);
      session.status(
        fakeCard(
          ready: false,
          blocked: Blocked.fault,
          fault: FaultView(
            feed: 7,
            outcome: FeedOutcome.failed,
            reason: FeedReason.jammed,
            result: fakeFed(
              confirmed: 1,
              outcome: FeedOutcome.failed,
              reason: FeedReason.jammed,
              atMs: _todayAt(7, 30),
            ),
          ),
        ),
      );
      await settle(tester);
      return session;
    }

    testWidgets('holds feeding, says what stopped it, and offers a restart', (
      tester,
    ) async {
      final session = await stopped(tester);

      expect(find.text("Can't feed"), findsOneWidget);
      expect(find.text('Meals are held until it restarts'), findsOneWidget);
      expect(find.text('The wheel jammed at 7:30 AM'), findsOneWidget);
      expect(
        find.textContaining('It stopped after 1 of 2 portions.'),
        findsOneWidget,
      );
      final feed = find.widgetWithText(
        FilledButton,
        'Feed now · unavailable until restart',
      );
      expect(tester.widget<FilledButton>(feed).onPressed, isNull);

      await _tap(tester, find.text('Restart it from here'));
      expect(session.requests, isEmpty, reason: 'it asks first');
      await _tap(tester, find.widgetWithText(FilledButton, 'Restart'));
      expect(session.requests, [const FeederRequest.reboot()]);
      expect(find.text('Kitchen is restarting.'), findsOneWidget);
    });

    testWidgets('a restart refused or unanswered says so', (tester) async {
      final session = await stopped(tester);
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.busy()))
        ..add(const FeederReply.unanswered());

      await _tap(tester, find.text('Restart it from here'));
      await _tap(tester, find.widgetWithText(FilledButton, 'Restart'));
      expect(find.textContaining("It's busy"), findsOneWidget);

      await _tap(tester, find.text('Restart it from here'));
      await _tap(tester, find.widgetWithText(FilledButton, 'Restart'));
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(find.text('Kitchen is restarting.'), findsNothing);
      expect(session.requests, hasLength(2));
    });
  });

  testWidgets('the lights help names every light and button', (tester) async {
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);
    session.status(fakeCard());
    await settle(tester);

    await _tap(tester, find.text('What do the lights on the feeder mean?'));

    expect(find.text('Lights and buttons'), findsOneWidget);
    for (final mark in ['A', 'B', 'C', 'D', '1', '2', '3', '4']) {
      expect(find.text(mark), findsNWidgets(2), reason: mark);
    }
  });

  testWidgets('the restart is in the menu too', (tester) async {
    final (fleet, session) = await _fleet(tester);
    await _showShell(tester, fleet);
    session.status(fakeCard());
    await settle(tester);

    await _tap(tester, find.byTooltip('More'));
    await _tap(tester, find.text('Restart'));
    await _tap(tester, find.widgetWithText(FilledButton, 'Restart'));
    expect(session.requests, [const FeederRequest.reboot()]);
  });

  group('Home', () {
    Future<(FeederFleet, FakeSession)> home(
      WidgetTester tester, {
      PendingInstall earlier = PendingInstall.nothing,
    }) async {
      _phone(tester);
      final (fleet, session) = await _fleet(tester);
      final router = wellfedRouter(
        book: FakeBook(),
        root: (context) =>
            HomeScreen(fleet: fleet, earlierInstall: () async => earlier),
        pages: const {},
        feeder: (context, identity, tab) =>
            FeederShell.of(fleet, identity, initialTab: tab),
      );
      addTearDown(router.dispose);
      await tester.pumpWidget(
        MaterialApp.router(
          theme: wellfedTheme(Brightness.light),
          routerConfig: router,
        ),
      );
      await settle(tester);
      return (fleet, session);
    }

    testWidgets('says when an earlier install did not finish', (tester) async {
      await home(tester);
      expect(find.text('Finish installing'), findsNothing);

      await home(tester, earlier: PendingInstall.values.last);
      expect(find.text('Finish installing'), findsOneWidget);
    });

    testWidgets('a last feed with no time says so', (tester) async {
      final (_, session) = await home(tester);
      session.status(
        fakeCard(
          lastFed: fakeFed(
            confirmed: 0,
            atMost: 2,
            outcome: FeedOutcome.uncertain,
            reason: FeedReason.interrupted,
          ),
        ),
      );
      await settle(tester);

      expect(find.text('Time unknown'), findsOneWidget);
      expect(find.text('Unknown'), findsNothing);
      expect(find.text('0 to 2 of 2'), findsOneWidget);
    });

    testWidgets("shows the next meal, the last feed and today's meals", (
      tester,
    ) async {
      final (_, session) = await home(tester);
      final now = DateTime.now();
      session.status(
        fakeCard(
          nextMeal: NextMealView(
            atMs: now.add(const Duration(minutes: 26)).millisecondsSinceEpoch,
            minute: 12 * 60 + 30,
            portions: 2,
          ),
          lastFed: fakeFed(atMs: _todayAt(7, 30)),
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
                atMs: now
                    .add(const Duration(minutes: 26, seconds: 30))
                    .millisecondsSinceEpoch,
              ),
            ),
            const MealView(
              slot: 2,
              minute: 18 * 60 + 30,
              portions: 3,
              call: 1,
              standing: MealStanding.later(),
            ),
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

    testWidgets('the card opens the feeder, and Schedule its tab', (
      tester,
    ) async {
      final (_, session) = await home(tester);
      session.status(
        fakeCard(
          meals: const [
            MealView(
              slot: 2,
              minute: 18 * 60 + 30,
              portions: 3,
              call: 0,
              standing: MealStanding.later(),
            ),
          ],
        ),
      );
      await settle(tester);

      await _tap(tester, find.widgetWithText(TextButton, 'Schedule'));
      final tabs = DefaultTabController.of(tester.element(find.byType(TabBar)));
      expect(tabs.index, 1);
    });

    testWidgets("a feeder with no clock offers this phone's", (tester) async {
      final (_, session) = await home(tester);
      session.status(fakeCard(clockSet: false));
      await settle(tester);

      expect(
        find.textContaining('Meals are on hold.', findRichText: true),
        findsOneWidget,
      );
      await _tap(tester, find.text('Set the time from this phone'));
      expect(session.requests, [const FeederRequest.syncClock()]);
    });
  });

  group('Feed now', () {
    Future<FakeSession> sheet(WidgetTester tester) async {
      final (fleet, session) = await _fleet(tester);
      await _showShell(tester, fleet);
      session.status(fakeCard());
      await settle(tester);
      await _tap(tester, find.widgetWithText(FilledButton, 'Feed now'));
      return session;
    }

    testWidgets('goes out once, and a refusal offers a new start', (
      tester,
    ) async {
      final session = await sheet(tester);
      expect(find.textContaining("Today's meals still run."), findsOneWidget);

      await _tap(tester, find.text('Feed 1 portion'));
      expect(session.feeds, [1]);

      session.updates.add(
        const SessionUpdate.feedRefused(refusal: FeedRefusal.busy),
      );
      await settle(tester);
      expect(find.text("Kitchen didn't feed"), findsOneWidget);
      expect(find.text('Start over'), findsOneWidget);
    });

    testWidgets('an unanswered feed is never offered again', (tester) async {
      final session = await sheet(tester);
      await _tap(tester, find.text('Feed 1 portion'));

      session.updates.add(const SessionUpdate.feedUnanswered());
      await settle(tester);
      expect(find.text('Did Kitchen feed?'), findsOneWidget);
      expect(find.text('Start over'), findsNothing);
      expect(session.feeds, [1]);
    });

    testWidgets('Feed now after a known result, tapped away, asks portions', (
      tester,
    ) async {
      final session = await sheet(tester);
      await _tap(tester, find.text('Feed 1 portion'));
      session.updates.add(
        const SessionUpdate.feedStarted(feed: 7, portions: 2, dueSeconds: 49),
      );
      await settle(tester);
      session.updates.add(SessionUpdate.feedEnded(result: fakeFed(feed: 7)));
      await settle(tester);
      expect(find.text('Fed 2 of 2 portions'), findsOneWidget);

      await tester.tapAt(const Offset(10, 10));
      await settle(tester);
      expect(find.text('Fed 2 of 2 portions'), findsNothing);

      await _tap(tester, find.text('Feed now'));
      expect(find.text('Fed 2 of 2 portions'), findsNothing);
      expect(find.text('Feed 1 portion'), findsOneWidget);
    });

    testWidgets(
      'Feed now after an unknown result, tapped away, still says so',
      (tester) async {
        final session = await sheet(tester);
        await _tap(tester, find.text('Feed 1 portion'));
        session.updates.add(const SessionUpdate.feedUnanswered());
        await settle(tester);
        expect(find.text('Did Kitchen feed?'), findsOneWidget);

        await tester.tapAt(const Offset(10, 10));
        await settle(tester);
        await _tap(tester, find.text('Feed now'));

        expect(find.text('Did Kitchen feed?'), findsOneWidget);
        expect(find.text('Feed 1 portion'), findsNothing);
        expect(session.feeds, [1]);
      },
    );

    testWidgets('an uncertain feed shows a range, and the history', (
      tester,
    ) async {
      final session = await sheet(tester);
      await _tap(tester, find.text('Feed 1 portion'));
      session.updates.add(
        const SessionUpdate.feedStarted(feed: 7, portions: 2, dueSeconds: 49),
      );
      await settle(tester);
      session.updates.add(
        SessionUpdate.feedEnded(
          result: fakeFed(
            confirmed: 0,
            atMost: 2,
            outcome: FeedOutcome.uncertain,
            reason: FeedReason.power,
          ),
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
        final book = FakeBook()..feeders = [_kitchen];
        final ports = FakeFeederPorts();
        final fleet = FeederFleet(book, ports);
        addTearDown(fleet.dispose);
        await tester.pump();
        ports.session.opened();
        await tester.pump();
        await _showShell(tester, fleet);
        ports.session.status(fakeCard());
        await settle(tester);
        await _tap(tester, find.widgetWithText(FilledButton, 'Feed now'));
        await _tap(tester, find.byTooltip('One more portion'));
        await _tap(tester, find.text('Feed 2 portions'));
        ports.session.updates.add(
          const SessionUpdate.feedStarted(feed: 7, portions: 2, dueSeconds: 60),
        );
        // One pump delivers it, so the due time counts from here exactly.
        await tester.pump();
        expect(find.text('Feeding Kitchen'), findsOneWidget);
        return ports;
      }

      const didFeed = 'Did Kitchen feed?';
      const justBefore = Duration(seconds: 59);

      testWidgets('a healthy slow feed reaches its result without going lost', (
        tester,
      ) async {
        final ports = await running(tester);
        await tester.pump(justBefore);
        expect(find.text('Feeding Kitchen'), findsOneWidget);

        ports.session.updates.add(
          SessionUpdate.feedEnded(result: fakeFed(feed: 7)),
        );
        await settle(tester);
        expect(find.text('Fed 2 of 2 portions'), findsOneWidget);
      });

      testWidgets(
        'an overdue feed goes lost, and its late result replaces it',
        (tester) async {
          final ports = await running(tester);
          await tester.pump(justBefore + const Duration(seconds: 2));
          await settle(tester);
          expect(find.text(didFeed), findsOneWidget);
          expect(find.text('Stop feeding'), findsNothing);
          expect(
            find.textContaining(
              "hasn't said how it ended, past the longest a feed can take.",
            ),
            findsOneWidget,
          );
          expect(find.textContaining('connection dropped'), findsNothing);
          expect(
            find.text(
              "Wellfed won't send it again on its own. Look at the bowl before "
              'you feed again.',
            ),
            findsOneWidget,
          );

          // Another feed's end says nothing about this one.
          ports.session.updates.add(
            SessionUpdate.feedEnded(result: fakeFed(feed: 8)),
          );
          await settle(tester);
          expect(find.text(didFeed), findsOneWidget);

          ports.session.updates.add(
            SessionUpdate.feedEnded(result: fakeFed(feed: 7)),
          );
          await settle(tester);
          expect(find.text('Fed 2 of 2 portions'), findsOneWidget);
        },
      );

      testWidgets('one feed goes out across going lost and reconnecting', (
        tester,
      ) async {
        final ports = await running(tester);
        final first = ports.session;

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
              result: fakeFed(
                feed: 7,
                confirmed: 0,
                atMost: 2,
                outcome: FeedOutcome.uncertain,
                reason: FeedReason.power,
              ),
            ),
          ),
        ]);
        await settle(tester);
        expect(find.text('Fed between 0 and 2 portions'), findsOneWidget);
        expect(ports.sessions.expand((session) => session.feeds), [2]);
      });

      testWidgets(
        'stop never says it stopped, and says when it could not ask',
        (tester) async {
          final ports = await running(tester);
          await _tap(tester, find.text('Stop feeding'));
          expect(ports.session.cancels, [7]);
          expect(find.text('Feeding Kitchen'), findsOneWidget);
          expect(
            find.text('Asked the feeder to stop. Waiting for it to answer.'),
            findsOneWidget,
          );

          ports.session.cancelFails = const BridgeError(
            kind: ErrorKind.closed,
            message: 'the session ended',
          );
          await _tap(tester, find.text('Stop feeding'));
          expect(find.text('Feeding Kitchen'), findsOneWidget);
          expect(
            find.textContaining('The feeder did not confirm the stop'),
            findsOneWidget,
          );

          ports.session.updates.add(
            SessionUpdate.feedEnded(result: fakeFed(feed: 7)),
          );
          await settle(tester);
        },
      );
    });
  });

  test('a request with no session open never went out', () async {
    final book = FakeBook()..feeders = [_kitchen];
    final ports = FakeFeederPorts();
    // Not opened yet in the first microtask: no session, nothing sent.
    final controller = FeederController(book, ports, _kitchen);
    final reply = await controller.ask(const FeederRequest.reboot());
    expect(reply, const FeederReply.notSent());
    controller.dispose();
  });
}
