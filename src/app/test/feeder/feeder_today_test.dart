import 'package:flutter/material.dart' hide Card;
import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'feeder_support.dart';

void main() {
  testWidgets('Today says how the feeder stands, as the feeder said it', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);

    final int fedAt = feederTodayAt(7, 30);
    session.status(
      fakeCard(
        nextMeal: NextMealView(slot: 0, atMs: feederTodayAt(12, 30), minute: 12 * 60 + 30, portions: 2),
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
    expect(find.textContaining(RegExp(r'^Next meal 12:30 PM · (in .+|now) · 2 portions$')), findsOneWidget);
    expect(find.textContaining(RegExp(r'^Last fed\s7:30 AM(\n| · )2 portions$')), findsOneWidget);
    expect(find.text('Food in'), findsOneWidget);
    expect(find.text('Buttons unlocked'), findsOneWidget);

    // Nothing needs the owner, so the facts fold into one line.
    expect(find.text('Strong'), findsNothing);
    await feederTap(tester, find.text('Power, Wi-Fi and clock look fine'));
    expect(find.text('Mains · batteries 90%'), findsOneWidget);
    expect(find.text('Strong'), findsOneWidget);
    expect(find.text('Set by your router at 11:58 AM'), findsOneWidget);
    expect(find.text('Change in 9 days'), findsOneWidget);
    expect(find.text('Mark changed'), findsNothing);
    await feederTap(tester, find.text('Show less'));
    expect(find.text('Strong'), findsNothing);
  });

  group('a clock with no zone and no offset', () {
    TimeInfo time({bool writable = true, String? zone, int? offsetS}) => TimeInfo(
      generation: 6,
      writable: writable,
      zone: zone,
      ntp: const NtpView.address(ip: '162.159.200.1', port: 123),
      syncs: 1,
      last: TimeSource.ntp,
      offsetS: offsetS,
    );

    testWidgets("says meals hold, and an owner's tap stores this phone's zone", (tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fakeCard(time: time()));
      await settle(tester);
      session.requests.clear();

      await tapIn(tester, find.text("Use this phone's zone"));
      final put = session.requests.first as FeederRequest_PutTime;
      expect(put.base, 6);
      expect(put.zone, const ZoneChange.phone());
      expect(session.requests.last, const FeederRequest.getTime());
    });

    testWidgets("a member's tap gives this phone's time instead", (tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fakeCard(time: time(writable: false)));
      await settle(tester);
      session.requests.clear();

      await tapIn(tester, find.text('Set the time from this phone'));
      expect(session.requests.first, const FeederRequest.syncClock());
    });

    testWidgets('says nothing once a zone or an offset is known', (tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fakeCard(time: time(zone: 'MST7')));
      await settle(tester);
      expect(find.text("Use this phone's zone"), findsNothing);

      session.status(fakeCard(time: time(offsetS: -25200)));
      await settle(tester);
      expect(find.text("Use this phone's zone"), findsNothing);
    });
  });

  group('a router that gives no time', () {
    TimeInfo time(NtpResult? last, {TimeSource? setBy}) => TimeInfo(
      generation: 4,
      writable: true,
      ntp: const NtpView.gateway(port: 123),
      syncs: 1,
      last: setBy,
      ntpLast: last,
    );

    testWidgets('offers Cloudflare, sent against the time generation', (tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fakeCard(time: time(NtpResult.lost)));
      await settle(tester);
      session.requests.clear();

      await tapIn(tester, find.widgetWithText(FilledButton, 'Use Cloudflare'));
      final put = session.requests.first as FeederRequest_PutTime;
      expect(put.base, 4);
      expect(put.ntp, '162.159.200.1');
      expect(session.requests.last, const FeederRequest.getTime());
    });

    testWidgets('is not offered for one lost answer from a router that set the clock', (tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fakeCard(time: time(NtpResult.lost, setBy: TimeSource.ntp)));
      await settle(tester);

      expect(find.text('Use Cloudflare'), findsNothing);
    });

    testWidgets('goes once the feeder reads back the new source', (tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fakeCard(time: time(NtpResult.lost)));
      await settle(tester);
      expect(find.text('Use Cloudflare'), findsOneWidget);

      session.status(
        fakeCard(
          time: const TimeInfo(
            generation: 5,
            writable: true,
            ntp: NtpView.address(ip: '162.159.200.1', port: 123),
            syncs: 1,
          ),
        ),
      );
      await settle(tester);
      expect(find.text('Use Cloudflare'), findsNothing);
    });

    testWidgets('is not offered while the router answers', (tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fakeCard(time: time(NtpResult.used)));
      await settle(tester);

      expect(find.text('Use Cloudflare'), findsNothing);
    });
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

    expect(find.textContaining(RegExp(r'^Last fed\sYesterday 9:00 PM(\n| · )2 portions$')), findsOneWidget);
    await feederTap(tester, find.text('Power, Wi-Fi and clock look fine'));
    expect(find.text('Set by a paired device yesterday at 9:00 PM'), findsOneWidget);
  });

  testWidgets('each fact is its own node, and the lock its own button', (tester) async {
    final SemanticsHandle semantics = tester.ensureSemantics();
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);
    session.status(fakeCard());
    await settle(tester);
    await feederTap(tester, find.text('Power, Wi-Fi and clock look fine'));

    final SemanticsNode power = tester.getSemantics(find.text('Power'));
    expect(power.label, contains('Power'));
    expect(power.label, isNot(contains('Wi-Fi')));
    expect(power.label, isNot(contains('Buttons')));
    expect(tester.getSemantics(find.text('Wi-Fi')), isNot(same(power)));

    final SemanticsNode lock = tester.getSemantics(find.text('Buttons unlocked'));
    expect(lock, isSemantics(label: 'Buttons unlocked', isButton: true, hasTapAction: true));
    expect(lock.hintOverrides?.onTapHint, 'Lock the buttons');
    expect(lock.rect.height, greaterThanOrEqualTo(48));

    semantics.dispose();
  });

  testWidgets('a low reading says low, and never that the hopper is empty', (tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);

    session.status(fakeCard(food: FoodLevel.low));
    await settle(tester);

    expect(find.text('Food low'), findsOneWidget);
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

    expect(find.text('Food low'), findsOneWidget);
    expect(find.textContaining('low at'), findsNothing);
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

    expect(find.textContaining(RegExp(r'^Last fed(\n| · )0 to 2 of 2 portions$')), findsOneWidget);
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

      await feederTap(tester, find.text('Buttons unlocked'));
      expect(session.requests, [const FeederRequest.setLock(locked: true)]);
      // Not moved by asking: only the status says it is locked.
      expect(find.text('Buttons unlocked'), findsOneWidget);

      session.status(fakeCard(locked: true));
      await settle(tester);
      expect(find.text('Buttons locked'), findsOneWidget);
    });

    testWidgets('says why it was refused', (tester) async {
      final FakeSession session = await locked(tester);
      session.replies.add(const FeederReply.refused(refusal: FeederRefusal.forbidden()));

      await feederTap(tester, find.text('Buttons unlocked'));
      expect(find.text("This phone isn't allowed to do that here."), findsOneWidget);
    });

    testWidgets('says an unanswered change is unknown, and sends it once', (tester) async {
      final FakeSession session = await locked(tester);
      session.replies.add(const FeederReply.unanswered());

      await feederTap(tester, find.text('Buttons unlocked'));
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
      await feederTap(tester, find.text('Mark changed'));
      expect(session.requests, [const FeederRequest.resetDesiccant()]);
    });

    testWidgets('says why the feeder refused it, or that it may have run', (tester) async {
      final FakeSession session = await due(tester);
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.noClock()))
        ..add(const FeederReply.unanswered());

      await feederTap(tester, find.text('Mark changed'));
      expect(find.textContaining('Set its time first'), findsOneWidget);

      await feederTap(tester, find.text('Mark changed'));
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
