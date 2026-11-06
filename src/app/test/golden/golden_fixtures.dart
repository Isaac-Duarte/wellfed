import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';

import '../support/fakes.dart';
import '../support/harness.dart';

// The feeder states more than one picture draws: the goldens and the owner
// guide's screenshots show the same day, so they cannot drift apart.

/// Four meals with a meal call each, one in every standing, so the rows show
/// with both a short time ("7:00 AM") and a long one ("12:00 PM").
Card fourMealsCard() => fakeCard(
  nextMeal: NextMealView(slot: 2, atMs: DateTime(2026, 11, 2, 17).millisecondsSinceEpoch, minute: 17 * 60, portions: 2),
  lastFed: fixtureFed,
  meals: [
    MealView(
      slot: 0,
      minute: 7 * 60,
      portions: 2,
      call: 1,
      standing: MealStanding.fed(result: fixtureFed),
    ),
    const MealView(slot: 1, minute: 12 * 60, portions: 1, call: 1, standing: MealStanding.passed()),
    MealView(
      slot: 2,
      minute: 17 * 60,
      portions: 2,
      call: 1,
      standing: MealStanding.next(atMs: DateTime(2026, 11, 2, 17).millisecondsSinceEpoch),
    ),
    const MealView(slot: 3, minute: 22 * 60, portions: 1, call: 1, standing: MealStanding.later()),
  ],
  plans: fullCard().plans,
);

/// What Today reads beside [fourMealsCard]: a press of the Dispense button
/// between the meals, and the noon meal missed.
List<HistoryEntry> fourMealsHistory() => [
  HistoryEntry(
    seq: BigInt.one,
    atMs: DateTime(2026, 11, 2, 9, 12).millisecondsSinceEpoch,
    body: HistoryBody.fed(
      result: fakeFed(atMs: DateTime(2026, 11, 2, 9, 12).millisecondsSinceEpoch, requested: 1, confirmed: 1),
    ),
  ),
  HistoryEntry(
    seq: BigInt.two,
    atMs: DateTime(2026, 11, 2, 12, 1).millisecondsSinceEpoch,
    body: HistoryBody.missed(
      slot: 1,
      dueMs: DateTime(2026, 11, 2, 12).millisecondsSinceEpoch,
      reason: const MissedReason.late_(),
    ),
  ),
];

/// Two days of a feeder's history as the owner meets it: the day's feeds, a
/// missed meal, and a burst of settings saves that folds into one row.
List<HistoryEntry> twoDaysHistory() {
  var seq = 0;
  HistoryEntry at(DateTime when, HistoryBody body) =>
      HistoryEntry(seq: BigInt.from(seq++), atMs: when.millisecondsSinceEpoch, body: body);
  FeedResult fed(DateTime when, [FedBy by = const FedBy.plan(slot: 0)]) =>
      fakeFed(atMs: when.millisecondsSinceEpoch, by: by);
  final today = DateTime(2026, 11, 2);
  final DateTime yesterday = today.subtract(const Duration(days: 1));
  return [
    at(yesterday.add(const Duration(hours: 7)), HistoryBody.fed(result: fed(yesterday.add(const Duration(hours: 7))))),
    at(
      yesterday.add(const Duration(hours: 12)),
      HistoryBody.missed(
        slot: 1,
        dueMs: yesterday.add(const Duration(hours: 12)).millisecondsSinceEpoch,
        reason: const MissedReason.late_(),
      ),
    ),
    at(
      yesterday.add(const Duration(hours: 17)),
      HistoryBody.fed(result: fed(yesterday.add(const Duration(hours: 17)))),
    ),
    for (var m = 0; m < 5; m++)
      at(
        yesterday.add(Duration(hours: 21, minutes: 20 + m)),
        const HistoryBody.settingsChanged(by: FedBy.client(fingerprint: '01')),
      ),
    at(yesterday.add(const Duration(hours: 22, minutes: 59)), const HistoryBody.clockSet(by: FedBy.feeder())),
    at(today.add(const Duration(minutes: 4)), const HistoryBody.joined(fingerprint: '02')),
    at(today.add(const Duration(hours: 7)), HistoryBody.fed(result: fed(today.add(const Duration(hours: 7))))),
    at(
      today.add(const Duration(hours: 9, minutes: 12)),
      HistoryBody.fed(result: fed(today.add(const Duration(hours: 9, minutes: 12)), const FedBy.panel())),
    ),
    at(today.add(const Duration(hours: 12)), HistoryBody.fed(result: fed(today.add(const Duration(hours: 12))))),
    at(today.add(const Duration(hours: 14)), const HistoryBody.food(level: FoodLevel.low)),
  ];
}

/// The fixture's two meals: breakfast at 8 every day, and dinner at 6 paused.
final twoPlans = PlansView(
  generation: 3,
  writable: true,
  plans: [
    const PlanView(minute: 8 * 60, portions: 2, days: Weekday.values, enabled: true, call: 0),
    const PlanView(minute: 18 * 60, portions: 1, days: Weekday.values, enabled: false, call: 2),
    ...List<PlanView?>.filled(8, null),
  ],
);
