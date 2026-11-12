import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/widgets/when.dart';

FeedResult _fed(FeedOutcome outcome, {required int confirmed, bool uncertain = false}) => FeedResult(
  feed: 1,
  requested: 2,
  confirmed: confirmed,
  atMost: 2,
  outcome: outcome,
  reason: FeedReason.done,
  uncertain: uncertain,
  words: '',
  by: const FedBy.unknown(),
  hopper: FoodLevel.unread,
);

void main() {
  test('a feed is full only when it ran to its end and gave all it was asked', () {
    for (final FeedOutcome outcome in FeedOutcome.values) {
      for (final int confirmed in [1, 2]) {
        final FedKind expected = outcome == FeedOutcome.done && confirmed == 2 ? FedKind.full : FedKind.short;
        expect(fedKind(_fed(outcome, confirmed: confirmed)), expected, reason: '$outcome, $confirmed of 2');
      }
    }
    expect(fedKind(_fed(FeedOutcome.uncertain, confirmed: 2, uncertain: true)), FedKind.unsure);
  });

  test('a reminder one day past says day, and more say days', () {
    expect(overdueText(1), '1 day overdue');
    expect(overdueText(2), '2 days overdue');
  });
}
