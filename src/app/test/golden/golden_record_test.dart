import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/live_log.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'golden_fixtures.dart';
import 'golden_support.dart';

void main() {
  setUpGoldens('golden_record_test.dart');

  for (final Brightness brightness in Brightness.values) {
    group(brightness.name, () {
      golden('history', (tester) async {
        final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
        answerAsFull(session);
        session.status(fullCard());
        session.history(twoDaysHistory());
        await tester.pump();
        await showPhone(tester, shell(feeder, historyTab), brightness);
        await matchesGolden('history', brightness);
      });

      golden('the live log', (tester) async {
        final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
        await showPhone(tester, overBlank(LiveLogScreen(feeder: feeder)), brightness);
        session.updates
          ..add(
            const SessionUpdate.log(
              seq: 40,
              dropped: 0,
              lines: [
                'PLNET1 rssi=-58 channel=6',
                'PLTIME1 sync source=ntp error_ms=12',
                'PLFD1 start n=2 by=plan slot=0',
                'PLMOT1 turn ms=812 current_ma=310',
                'PLMOT1 turn ms=798 current_ma=305',
                'PLFD1 done n=2 confirmed=2',
              ],
            ),
          )
          ..add(
            const SessionUpdate.log(seq: 52, dropped: 6, lines: ['PLNET1 rssi=-61 channel=6', 'PLHOP1 level=present']),
          );
        await settle(tester);
        await matchesGolden('live_log', brightness);
      });
    });
  }
}
