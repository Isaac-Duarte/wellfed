import 'package:flutter/foundation.dart' show Uint8List;
import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/feed_detail.dart';
import 'package:wellfed/screens/feeder/history.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/widgets/icons.dart';

import '../support/fakes.dart';
import '../support/harness.dart';

void main() {
  group('History', () {
    HistoryEntry entry(int seq, HistoryBody body, {DateTime? at}) =>
        HistoryEntry(seq: BigInt.from(seq), atMs: at?.millisecondsSinceEpoch, body: body);

    Future<(FeederController, FakeSession)> show(WidgetTester tester, List<HistoryEntry> entries) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.replies.add(
        FeederReply.clients(
          clients: [
            ClientView(
              key: Uint8List(0),
              fingerprint: 'beef',
              label: "Ana's phone",
              role: ClientRole.member,
              you: false,
            ),
          ],
        ),
      );
      session.history(entries);
      await showTab(tester, HistoryTab(feeder: feeder));
      return (feeder, session);
    }

    final now = DateTime.now();

    testWidgets('shows what happened, newest first, under its day', (tester) async {
      await show(tester, [
        entry(
          1,
          const HistoryBody.missed(slot: 1, dueMs: 0, reason: MissedReason.blocked(reason: Blocked.battery)),
          at: now.subtract(const Duration(days: 1)),
        ),
        entry(2, HistoryBody.gap(from: BigInt.from(3), to: BigInt.from(5))),
        entry(3, const HistoryBody.restarted(), at: now),
        entry(
          4,
          HistoryBody.fed(result: fakeFed(atMs: now.millisecondsSinceEpoch).copyByClient('beef')),
          at: now,
        ),
      ]);

      expect(find.text('Today'), findsOneWidget);
      expect(find.text('Yesterday'), findsOneWidget);
      expect(find.text('Fed 2 of 2'), findsOneWidget);
      expect(find.text("Feed now · Ana's phone"), findsOneWidget);
      expect(find.text('Started up'), findsOneWidget);
      expect(find.textContaining('the batteries were too low'), findsOneWidget);
      expect(find.text("Some events here weren't kept"), findsOneWidget);
      final double fed = tester.getTopLeft(find.text('Fed 2 of 2')).dy;
      final double started = tester.getTopLeft(find.text('Started up')).dy;
      expect(fed, lessThan(started));
    });

    testWidgets('a readiness change reads as one, not a refused feed', (tester) async {
      await show(tester, [
        entry(1, const HistoryBody.feedingBlocked(reason: Blocked.setup), at: now),
        entry(2, const HistoryBody.feedingBlocked(), at: now),
        entry(3, const HistoryBody.dispenseRefused(reason: Blocked.fault), at: now),
      ]);

      expect(find.text('Not ready to feed'), findsOneWidget);
      expect(find.text('In setup mode'), findsOneWidget);
      expect(find.text('Ready to feed again'), findsOneWidget);
      expect(find.text("Couldn't feed"), findsNothing);
      expect(find.text('Can feed again'), findsNothing);
      expect(find.text('A press was refused'), findsOneWidget);
    });

    testWidgets('setup mode is not a problem, and shows under All', (tester) async {
      await show(tester, [
        entry(1, const HistoryBody.feedingBlocked(reason: Blocked.setup), at: now),
        entry(2, const HistoryBody.feedingBlocked(reason: Blocked.updating), at: now),
      ]);

      expect(find.text('In setup mode'), findsOneWidget);
      expect(find.text('Installing an update'), findsOneWidget);
      final Finder setup = find.ancestor(of: find.text('In setup mode'), matching: find.byType(Row));
      final Finder other = find.ancestor(of: find.text('Installing an update'), matching: find.byType(Row));
      Glyph glyph(Finder row) =>
          tester.widget<StrokeIcon>(find.descendant(of: row, matching: find.byType(StrokeIcon))).glyph;
      expect(glyph(setup), Glyph.info);
      expect(glyph(other), Glyph.warning);

      await tapIn(tester, find.widgetWithText(ChoiceChip, 'Problems'));
      expect(find.text('In setup mode'), findsNothing);
      expect(find.text('Installing an update'), findsOneWidget);
    });

    testWidgets('an uncertain feed of no known source says interrupted', (tester) async {
      final FeedResult uncertain = fakeFed(
        confirmed: 0,
        atMost: 2,
        outcome: FeedOutcome.uncertain,
        reason: FeedReason.interrupted,
        by: const FedBy.unknown(),
      );
      await show(tester, [entry(1, HistoryBody.fed(result: uncertain), at: now)]);

      expect(find.text('Fed between 0 and 2'), findsOneWidget);
      expect(find.text('Interrupted · not repeated'), findsOneWidget);
      expect(find.textContaining('Unknown'), findsNothing);
    });

    testWidgets('an uncertain feed is a range, also in its detail', (tester) async {
      final FeedResult uncertain = fakeFed(
        confirmed: 0,
        atMost: 3,
        requested: 3,
        outcome: FeedOutcome.uncertain,
        reason: FeedReason.power,
        atMs: now.millisecondsSinceEpoch,
      );
      await show(tester, [entry(1, HistoryBody.fed(result: uncertain), at: now)]);

      expect(find.text('Fed between 0 and 3'), findsOneWidget);
      await tapIn(tester, find.text('Fed between 0 and 3'));
      expect(find.byType(FeedDetail), findsOneWidget);
      expect(find.text('Fed between 0 and 3 portions'), findsOneWidget);
      expect(find.text('3 asked for · none confirmed · each could have dropped'), findsOneWidget);
      expect(find.text('Motor trace for this feed'), findsOneWidget);
    });

    testWidgets('filters by kind', (tester) async {
      await show(tester, [
        entry(1, HistoryBody.fed(result: fakeFed()), at: now),
        entry(2, const HistoryBody.plansChanged(by: FedBy.feeder()), at: now),
        entry(3, const HistoryBody.lost(count: 2), at: now),
      ]);

      await tapIn(tester, find.widgetWithText(ChoiceChip, 'Feeds'));
      expect(find.text('Fed 2 of 2'), findsOneWidget);
      expect(find.text('Meals changed'), findsNothing);

      await tapIn(tester, find.widgetWithText(ChoiceChip, 'Changes'));
      expect(find.text('Meals changed'), findsOneWidget);
      expect(find.text('Fed 2 of 2'), findsNothing);

      await tapIn(tester, find.widgetWithText(ChoiceChip, 'Problems'));
      expect(find.text("Some events weren't kept"), findsOneWidget);
      expect(find.text('Fed 2 of 2'), findsNothing);
    });
  });
}
