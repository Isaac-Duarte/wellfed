import 'package:flutter/material.dart' hide Card;
import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';
import 'feeder_support.dart';

void main() {
  testWidgets('Today says how the feeder stands, as the feeder said it', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);

    final int fedAt = feederTodayAt(7, 30);
    session.status(
      fakeCard(
        nextMeal: NextMealView(atMs: feederTodayAt(12, 30), minute: 12 * 60 + 30, portions: 2),
        lastFed: fakeFed(atMs: fedAt),
        time: TimeInfo(
          generation: 1,
          writable: true,
          ntp: const NtpView.gateway(port: 123),
          syncs: 3,
          last: TimeSource.ntp,
          lastAtMs: feederTodayAt(11, 58),
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
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);

    final DateTime yesterday = DateTime.now().subtract(const Duration(days: 1));
    final int at = DateTime(yesterday.year, yesterday.month, yesterday.day, 21).millisecondsSinceEpoch;
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
    expect(find.text('Set by a paired device yesterday at 9:00 PM'), findsOneWidget);
  });

  testWidgets('each fact is its own node, and the lock its own switch', (tester) async {
    final SemanticsHandle semantics = tester.ensureSemantics();
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);
    session.status(fakeCard());
    await settle(tester);

    final SemanticsNode food = tester.getSemantics(find.text('Food'));
    expect(food.label, contains('Food'));
    expect(food.label, isNot(contains('Wi-Fi')));
    expect(food.label, isNot(contains('Lock the buttons')));
    expect(tester.getSemantics(find.text('Wi-Fi')), isNot(same(food)));

    final SemanticsNode lock = tester.getSemantics(find.byType(Switch));
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
    final Size bounds = tester.getSize(find.byType(Switch));
    expect(lock.rect.width, lessThanOrEqualTo(bounds.width));

    semantics.dispose();
  });

  testWidgets('a low reading says low, and never that the hopper is empty', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);

    session.status(fakeCard(food: FoodLevel.low));
    await settle(tester);

    expect(find.textContaining('Low'), findsWidgets);
    expect(find.textContaining('empty'), findsNothing);
    expect(find.textContaining('Hopper'), findsNothing);
  });

  testWidgets('a low reading at rest borrows no time from the last feed', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);

    // The last feed read food; the level has gone low since, with no event.
    final int fedAt = feederTodayAt(7, 30);
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

  testWidgets('an uncertain last feed is a range, never one number', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);

    session.status(
      fakeCard(
        lastFed: fakeFed(confirmed: 0, atMost: 2, outcome: FeedOutcome.uncertain, reason: FeedReason.interrupted),
      ),
    );
    await settle(tester);

    expect(find.text('Last fed 0 to 2 of 2'), findsOneWidget);
  });

  testWidgets('a one-portion feed that stopped says portion, not portions', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);

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

    expect(find.textContaining('It stopped somewhere between 0 and 1 of 1 portion.'), findsOneWidget);
  });

  testWidgets('a feed that stopped unsure says the range in words', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);

    session.status(
      fakeCard(
        fault: FaultView(
          feed: 7,
          outcome: FeedOutcome.uncertain,
          reason: FeedReason.power,
          result: fakeFed(confirmed: 0, atMost: 2, outcome: FeedOutcome.uncertain, reason: FeedReason.power),
        ),
      ),
    );
    await settle(tester);

    expect(find.textContaining('It stopped somewhere between 0 and 2 of 2 portions.'), findsOneWidget);
  });

  testWidgets("History's filter survives a tab away while the screen is open", (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);
    session.status(fakeCard());
    await settle(tester);

    bool problems() => tester.widget<ChoiceChip>(find.widgetWithText(ChoiceChip, 'Problems')).selected;

    await feederTap(tester, find.text('History'));
    expect(problems(), isFalse);
    // A plain tap: scrolling it into view would slide the tabs' page as well.
    await tester.tap(find.widgetWithText(ChoiceChip, 'Problems'));
    await settle(tester);
    expect(problems(), isTrue);

    await feederTap(tester, find.text('Today'));
    expect(find.widgetWithText(ChoiceChip, 'Problems'), findsNothing);
    await feederTap(tester, find.text('History'));
    expect(problems(), isTrue);
  });

  group('the lock', () {
    Future<FakeSession> locked(WidgetTester tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fakeCard());
      await settle(tester);
      return session;
    }

    testWidgets('goes out once and follows what the feeder says', (tester) async {
      final FakeSession session = await locked(tester);

      await feederTap(tester, find.byType(Switch));
      expect(session.requests, [const FeederRequest.setLock(locked: true)]);
      // Not moved by asking: only the status says it is locked.
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);

      session.status(fakeCard(locked: true));
      await settle(tester);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
    });

    testWidgets('says why it was refused', (tester) async {
      final FakeSession session = await locked(tester);
      session.replies.add(const FeederReply.refused(refusal: FeederRefusal.forbidden()));

      await feederTap(tester, find.byType(Switch));
      expect(find.text("This phone isn't allowed to do that here."), findsOneWidget);
    });

    testWidgets('says an unanswered change is unknown, and sends it once', (tester) async {
      final FakeSession session = await locked(tester);
      session.replies.add(const FeederReply.unanswered());

      await feederTap(tester, find.byType(Switch));
      expect(find.textContaining('may or may not have'), findsOneWidget);
      await settle(tester);
      expect(session.requests, hasLength(1));
    });
  });

  group('the desiccant', () {
    Future<FakeSession> due(WidgetTester tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fakeCard(desiccant: const Desiccant.remaining(days: -3, due: true)));
      await settle(tester);
      return session;
    }

    testWidgets('records a change when it is due', (tester) async {
      final FakeSession session = await due(tester);

      expect(find.text('3 days overdue'), findsOneWidget);
      await feederTap(tester, find.text('Changed today'));
      expect(session.requests, [const FeederRequest.resetDesiccant()]);
    });

    testWidgets('says why the feeder refused it, or that it may have run', (tester) async {
      final FakeSession session = await due(tester);
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.noClock()))
        ..add(const FeederReply.unanswered());

      await feederTap(tester, find.text('Changed today'));
      expect(find.textContaining('Set its time first'), findsOneWidget);

      await feederTap(tester, find.text('Changed today'));
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(session.requests, hasLength(2));
    });
  });

  group('a fault', () {
    Future<FakeSession> stopped(WidgetTester tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
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
              atMs: feederTodayAt(7, 30),
            ),
          ),
        ),
      );
      await settle(tester);
      return session;
    }

    testWidgets('holds feeding, says what stopped it, and offers a restart', (tester) async {
      final FakeSession session = await stopped(tester);

      expect(find.text("Can't feed"), findsOneWidget);
      expect(find.text('Meals are held until it restarts'), findsOneWidget);
      expect(find.text('The wheel jammed at 7:30 AM'), findsOneWidget);
      expect(find.textContaining('It stopped after 1 of 2 portions.'), findsOneWidget);
      final Finder feed = find.widgetWithText(FilledButton, 'Feed now · unavailable until restart');
      expect(tester.widget<FilledButton>(feed).onPressed, isNull);

      await feederTap(tester, find.text('Restart it from here'));
      expect(session.requests, isEmpty, reason: 'it asks first');
      await feederTap(tester, find.widgetWithText(FilledButton, 'Restart'));
      expect(session.requests, [const FeederRequest.reboot()]);
      expect(find.text('Kitchen is restarting.'), findsOneWidget);
    });

    testWidgets('a restart refused or unanswered says so', (tester) async {
      final FakeSession session = await stopped(tester);
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.busy()))
        ..add(const FeederReply.unanswered());

      await feederTap(tester, find.text('Restart it from here'));
      await feederTap(tester, find.widgetWithText(FilledButton, 'Restart'));
      expect(find.textContaining("It's busy"), findsOneWidget);

      await feederTap(tester, find.text('Restart it from here'));
      await feederTap(tester, find.widgetWithText(FilledButton, 'Restart'));
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(find.text('Kitchen is restarting.'), findsNothing);
      expect(session.requests, hasLength(2));
    });
  });

  testWidgets('the lights help names every light and button', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);
    session.status(fakeCard());
    await settle(tester);

    await feederTap(tester, find.text('What do the lights on the feeder mean?'));

    expect(find.text('Lights and buttons'), findsOneWidget);
    for (final mark in ['A', 'B', 'C', 'D', '1', '2', '3', '4']) {
      expect(find.text(mark), findsNWidgets(2), reason: mark);
    }
  });

  testWidgets('the restart is in the menu too', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);
    session.status(fakeCard());
    await settle(tester);

    await feederTap(tester, find.byTooltip('More'));
    await feederTap(tester, find.text('Restart'));
    await feederTap(tester, find.widgetWithText(FilledButton, 'Restart'));
    expect(session.requests, [const FeederRequest.reboot()]);
  });
}
