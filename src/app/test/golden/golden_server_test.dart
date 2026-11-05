import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/server/account.dart';
import 'package:wellfed/screens/server/add_server.dart';
import 'package:wellfed/screens/server/feeder_food.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/server/add_server.dart';
import 'package:wellfed/state/server/feeder_food.dart';
import 'package:wellfed/state/server/server.dart';

import '../support/fakes.dart';
import 'golden_support.dart';

/// The food view of a feeder whose portion size is not set: portions only.
FakeServerSession _uncalibrated() => FakeServerSession()
  ..home.food = const ServerFeederFood(
    weight: PortionWeight.uncalibrated(),
    sinceRefill: ServerSinceRefill(portions: 14, uncertainPortions: 1),
    rate: FeedRate(portions: 6, hours: 24),
    anomalies: [],
  );

void main() {
  setUpGoldens('golden_server_test.dart');

  for (final Brightness brightness in Brightness.values) {
    group(brightness.name, () {
      golden('adding a home server', (tester) async {
        final server = HomeServer(FakeBook(), FakeServerPorts(), FakeRouter());
        addTearDown(server.dispose);
        await showPhone(tester, overBlank(AddServerScreen(newFlow: () => AddServerFlow(server))), brightness);
        await matchesGolden('server_add', brightness);
      });

      golden('the home server account', (tester) async {
        final server = HomeServer(FakeBook(), FakeServerPorts(), FakeRouter());
        addTearDown(server.dispose);
        await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
        await showPhone(tester, overBlank(AccountScreen(server: server, household: () {})), brightness);
        await matchesGolden('server_account', brightness);
      });

      golden("an uncalibrated feeder's food", (tester) async {
        await showPhone(
          tester,
          overBlank(FeederFoodScreen(flow: FeederFoodFlow(_uncalibrated(), 'id-k'), name: 'Kitchen')),
          brightness,
        );
        await matchesGolden('server_food_uncalibrated', brightness);
      });
    });
  }
}
