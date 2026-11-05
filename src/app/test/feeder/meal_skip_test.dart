import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/history.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'feeder_support.dart';

/// Supper (slot 0) still to come and offered for a skip, a late snack (slot 1)
/// skipped and offered for an undo, and breakfast (slot 2) skipped with its
/// time come. What each row offers is the card's: the screen only shows it.
Card skippingCard() {
  final int supper = feederTodayAt(18, 0);
  final int snack = feederTodayAt(23, 0);
  return fakeCard(
    meals: [
      const MealView(slot: 2, minute: 7 * 60, portions: 1, call: 0, standing: MealStanding.skipKept()),
      MealView(
        slot: 0,
        minute: 18 * 60,
        portions: 2,
        call: 0,
        standing: MealStanding.next(atMs: supper),
        action: MealAction.skip(dueMs: supper),
      ),
      MealView(
        slot: 1,
        minute: 23 * 60,
        portions: 1,
        call: 0,
        standing: MealStanding.skipped(atMs: snack),
        action: MealAction.undo(dueMs: snack),
      ),
    ],
  );
}

void main() {
  Future<FakeSession> today(WidgetTester tester) async {
    final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
    await feederShowShell(tester, fleet);
    session.status(skippingCard());
    await settle(tester);
    session.requests.clear();
    return session;
  }

  testWidgets('a skip asks first, then names the meal the card offered', (tester) async {
    final FakeSession session = await today(tester);
    expect(find.text('Skipped'), findsNWidgets(2));
    expect(find.textContaining('Missed'), findsNothing, reason: 'a skip is not a missed meal');
    expect(find.text('Skip'), findsOneWidget, reason: 'only the meal still to come');
    expect(find.text('Undo'), findsOneWidget, reason: 'only the skip still to come');

    await feederTap(tester, find.text('Skip'));
    expect(find.text('Skip the 6:00 PM meal today?'), findsOneWidget);
    await feederTap(tester, find.text('Keep it'));
    expect(session.requests, isEmpty, reason: 'kept, nothing is sent');

    await feederTap(tester, find.text('Skip'));
    await feederTap(tester, find.text('Skip meal'));
    final skip = session.requests.single as FeederRequest_SkipMeal;
    expect((skip.slot, skip.dueMs), (0, feederTodayAt(18, 0)));
    expect(find.byType(SnackBar), findsNothing);
  });

  testWidgets('an undo goes at once, and one too late says the skip stays', (tester) async {
    final FakeSession session = await today(tester);

    await feederTap(tester, find.text('Undo'));
    final undo = session.requests.single as FeederRequest_UnskipMeal;
    expect((undo.slot, undo.dueMs), (1, feederTodayAt(23, 0)));

    session.replies.add(const FeederReply.refused(refusal: FeederRefusal.invalid()));
    await feederTap(tester, find.text('Undo'));
    expect(find.textContaining('already due, so it stays skipped'), findsOneWidget);
    expect(session.requests, hasLength(2), reason: 'the refused undo went once, and is not sent again');
  });

  testWidgets('a refused skip says why in plain words', (tester) async {
    final FakeSession session = await today(tester);
    session.replies.add(const FeederReply.refused(refusal: FeederRefusal.busy()));

    await feederTap(tester, find.text('Skip'));
    await feederTap(tester, find.text('Skip meal'));
    expect(find.textContaining('busy with a feed or a change to the meals'), findsOneWidget);
  });

  testWidgets('History names a skip and its undo with who asked', (tester) async {
    final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
    final int at = feederTodayAt(9, 0);
    session.history([
      HistoryEntry(
        seq: BigInt.one,
        atMs: at,
        body: HistoryBody.skipped(
          slot: 0,
          dueMs: feederTodayAt(18, 0),
          by: const FedBy.client(fingerprint: 'beef'),
        ),
      ),
      HistoryEntry(
        seq: BigInt.two,
        atMs: at,
        body: HistoryBody.unskipped(slot: 0, dueMs: feederTodayAt(18, 0), by: const FedBy.feeder()),
      ),
    ]);
    await showTab(tester, HistoryTab(feeder: feeder));

    expect(find.text('Skipped the 6:00 PM meal'), findsOneWidget);
    expect(find.text('Brought back the 6:00 PM meal'), findsOneWidget);
    expect(find.text('The feeder'), findsOneWidget);
  });
}
