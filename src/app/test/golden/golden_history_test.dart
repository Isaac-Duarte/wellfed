import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'golden_support.dart';

/// Two days of a feeder's history as the owner meets it: the day's feeds, a
/// skipped meal, and a burst of settings saves that folds into one row.
List<HistoryEntry> _history() {
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

void main() {
  setUpGoldens('golden_history_test.dart');

  for (final Brightness brightness in Brightness.values) {
    group(brightness.name, () {
      golden('history', (tester) async {
        final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
        answerAsFull(session);
        session.status(fullCard());
        session.history(_history());
        await tester.pump();
        await showPhone(tester, shell(feeder, historyTab), brightness);
        await matchesGolden('history', brightness);
      });
    });
  }
}
