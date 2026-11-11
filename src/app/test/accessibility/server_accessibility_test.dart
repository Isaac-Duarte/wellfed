import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/home/adopt_prompt.dart';
import 'package:wellfed/screens/server/account.dart';
import 'package:wellfed/screens/server/add_server.dart';
import 'package:wellfed/screens/server/feeder_food.dart';
import 'package:wellfed/screens/server/household.dart';
import 'package:wellfed/screens/server/pet.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/adopt.dart';
import 'package:wellfed/state/server/add_server.dart';
import 'package:wellfed/state/server/feeder_food.dart';
import 'package:wellfed/state/server/household.dart';
import 'package:wellfed/state/server/pet.dart';
import 'package:wellfed/state/server/server.dart';

import '../support/accessibility.dart';
import '../support/fakes.dart';

const _kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');
const _cat = ServerPet(id: 'p1', name: 'Miso', species: Species.cat, feeders: ['id-k']);

Future<void> _show(WidgetTester tester, Widget screen, Brightness brightness) async {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(themed(screen, brightness: brightness));
  await settle(tester);
}

FakeServerSession _home() {
  final session = FakeServerSession();
  session.home
    ..pets = const [_cat]
    ..foods = const [ServerFood(id: 'f1', brand: 'Acme', name: 'Chicken', kcalPerKg: 3800, bagGrams: 2000)]
    ..weights = const [ServerWeight(id: 'w1', atMs: 0, grams: 4200)]
    ..food = const ServerFeederFood(
      weight: PortionWeight.uncalibrated(),
      sinceRefill: ServerSinceRefill(portions: 14, uncertainPortions: 0),
      anomalies: [],
    );
  return session;
}

void main() {
  for (final Brightness brightness in Brightness.values) {
    group('${brightness.name} theme', () {
      testWidgets('adding a home server', (tester) async {
        final server = HomeServer(FakeBook(), FakeServerPorts(), FakeRouter());
        addTearDown(server.dispose);
        await _show(tester, AddServerScreen(newFlow: () => AddServerFlow(server)), brightness);
        await expectAccessible(tester);
      });

      testWidgets('the home server account', (tester) async {
        final server = HomeServer(FakeBook(), FakeServerPorts(), FakeRouter());
        addTearDown(server.dispose);
        await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
        await _show(tester, AccountScreen(server: server, phoneKey: FakeClientKey(), household: () {}), brightness);
        await expectAccessible(tester);
      });

      testWidgets('pets and foods', (tester) async {
        final FakeServerSession session = _home();
        await _show(
          tester,
          HouseholdScreen(flow: HouseholdFlow(session), newPet: (_) => null, feeders: const []),
          brightness,
        );
        await expectAccessible(tester);
      });

      testWidgets('a pet', (tester) async {
        final FakeServerSession session = _home();
        final household = HouseholdFlow(session);
        addTearDown(household.dispose);
        await _show(
          tester,
          PetScreen(
            flow: PetFlow(session, _cat),
            household: household,
            feeders: const [ListedFeeder(id: 'id-k', identity: 'aa', name: 'Kitchen')],
          ),
          brightness,
        );
        await expectAccessible(tester);
      });

      testWidgets("a feeder's food", (tester) async {
        await _show(tester, FeederFoodScreen(flow: FeederFoodFlow(_home(), 'id-k'), name: 'Kitchen'), brightness);
        await expectAccessible(tester);
      });

      testWidgets('the adoption prompt', (tester) async {
        final server = HomeServer(FakeBook(), FakeServerPorts(), FakeRouter());
        final flow = AdoptFlow(FakeFeederPorts(), server);
        addTearDown(() {
          flow.dispose();
          server.dispose();
        });
        await _show(
          tester,
          Scaffold(
            body: AdoptPrompt(feeder: _kitchen, flow: flow),
          ),
          brightness,
        );
        await expectAccessible(tester);
      });
    });
  }
}
