import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/screens/server/account.dart';
import 'package:wellfed/screens/server/add_server.dart';
import 'package:wellfed/screens/server/feeder_food.dart';
import 'package:wellfed/screens/server/pet.dart';
import 'package:wellfed/screens/server/server_words.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/adopt.dart';
import 'package:wellfed/state/server/add_server.dart';
import 'package:wellfed/state/server/feeder_food.dart';
import 'package:wellfed/state/server/household.dart';
import 'package:wellfed/state/server/pet.dart';
import 'package:wellfed/state/server/records.dart';
import 'package:wellfed/state/server/server.dart';
import 'package:wellfed/state/server/server_history.dart';

import '../support/fakes.dart';

const _kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');
const _cat = ServerPet(id: 'p1', name: 'Miso', species: Species.cat, feeders: ['id-k']);

(HomeServer, FakeServerPorts, FakeBook) _server() {
  final book = FakeBook()..feeders = [_kitchen];
  final ports = FakeServerPorts();
  final server = HomeServer(book, ports, FakeRouter(), FakeClientKey());
  addTearDown(server.dispose);
  return (server, ports, book);
}

void _phone(WidgetTester tester) {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

HistoryEntry _entry(int seq) => HistoryEntry(seq: BigInt.from(seq), atMs: 1000 * seq, body: const HistoryBody.other());

void main() {
  testWidgets('adding a server offers what the LAN announces, and logs in at the address as typed', (tester) async {
    _phone(tester);
    final (HomeServer server, FakeServerPorts ports, FakeBook book) = _server();
    await tester.pumpWidget(themed(AddServerScreen(newFlow: () => AddServerFlow(server))));
    ports.found
      ..add(const FoundServer(url: 'http://192.168.1.20:8080', name: 'home-1a2b3c4d'))
      ..add(const FoundServer(url: 'http://192.168.1.20:8080', name: 'home-1a2b3c4d'));
    await settle(tester);
    expect(find.text('home-1a2b3c4d'), findsOneWidget, reason: 'heard twice, listed once');

    await tester.enterText(find.byType(TextField).at(0), '192.168.1.20:8080');
    await tester.enterText(find.byType(TextField).at(1), 'ada');
    await tester.enterText(find.byType(TextField).at(2), 'hunter2');
    await tester.pump();
    await tester.ensureVisible(find.text('Log in'));
    await tester.tap(find.text('Log in'));
    await settle(tester);

    expect(ports.logins.single.url, '192.168.1.20:8080', reason: 'Rust picks the scheme, never Dart');
    expect(book.server?.link, serverLink);
    expect(server.state, isA<ServerLinked>());
  });

  testWidgets('a login pops the add-server screen once, though a server is heard as it leaves', (tester) async {
    _phone(tester);
    final (HomeServer server, FakeServerPorts ports, _) = _server();
    await tester.pumpWidget(themed(const Text('beneath')));
    final NavigatorState navigator = tester.state(find.byType(Navigator));
    unawaited(
      navigator.push(MaterialPageRoute<bool>(builder: (_) => AddServerScreen(newFlow: () => AddServerFlow(server)))),
    );
    await settle(tester);

    await tester.enterText(find.byType(TextField).at(1), 'ada');
    await tester.enterText(find.byType(TextField).at(2), 'hunter2');
    await tester.enterText(find.byType(TextField).at(0), '192.168.1.20:8080');
    await tester.pump();
    await tester.ensureVisible(find.text('Log in'));
    await tester.tap(find.text('Log in'));
    await tester.pump();
    ports.found.add(const FoundServer(url: 'http://192.168.1.21:8080', name: 'other'));
    await settle(tester);

    expect(find.text('beneath'), findsOneWidget);
  });

  testWidgets('a plain address heard on the network says the password would go unencrypted', (tester) async {
    _phone(tester);
    final (HomeServer server, FakeServerPorts ports, _) = _server();
    await tester.pumpWidget(themed(AddServerScreen(newFlow: () => AddServerFlow(server))));
    ports.found.add(const FoundServer(url: 'http://192.168.1.20:8080', name: 'home-1a2b3c4d'));
    await settle(tester);

    expect(find.text('http://192.168.1.20:8080 · not encrypted'), findsOneWidget);
    expect(find.textContaining('Not encrypted.', findRichText: true), findsNothing, reason: 'nothing picked yet');
    await tester.tap(find.text('home-1a2b3c4d'));
    await tester.pump();
    expect(find.textContaining('Not encrypted.', findRichText: true), findsOneWidget);
  });

  testWidgets('plain http beyond the home network is refused, and says how to reach the server', (tester) async {
    _phone(tester);
    final (HomeServer server, FakeServerPorts ports, FakeBook book) = _server();
    ports.loginFails = serverError(ServerErrorKind.plainHttp);
    await tester.pumpWidget(themed(AddServerScreen(newFlow: () => AddServerFlow(server))));
    // The address last: a plain one adds a notice that moves the others.
    await tester.enterText(find.byType(TextField).at(1), 'ada');
    await tester.enterText(find.byType(TextField).at(2), 'hunter2');
    await tester.enterText(find.byType(TextField).at(0), 'http://feeder.example');
    await tester.pump();
    await tester.ensureVisible(find.text('Log in'));
    await tester.tap(find.text('Log in'));
    await settle(tester);

    expect(find.textContaining("Use the server's https:// address"), findsOneWidget);
    expect(book.server, isNull);
    expect(ports.kept, isNull);
  });

  // A login changes nothing a retry could repeat, so a server that failed
  // partway asks for another try, not a check.
  for (final (ServerErrorKind kind, String words) in [
    (ServerErrorKind.badCredentials, 'The name or the password is wrong.'),
    (ServerErrorKind.serverFailed, 'The server failed partway through. Try again in a while.'),
  ]) {
    testWidgets('a login refused as ${kind.name} says why, and keeps nothing', (tester) async {
      _phone(tester);
      final (HomeServer server, FakeServerPorts ports, FakeBook book) = _server();
      ports.loginFails = serverError(kind);
      await tester.pumpWidget(themed(AddServerScreen(newFlow: () => AddServerFlow(server))));
      // The address last: a plain one adds a notice that moves the others.
      await tester.enterText(find.byType(TextField).at(1), 'ada');
      await tester.enterText(find.byType(TextField).at(2), 'wrong');
      await tester.enterText(find.byType(TextField).at(0), 'http://home.lan:8080');
      await tester.pump();
      await tester.ensureVisible(find.text('Log in'));
      await tester.tap(find.text('Log in'));
      await settle(tester);

      expect(find.text(words), findsOneWidget);
      expect(book.server, isNull);
    });
  }

  testWidgets('the account says the phone keeps its keys, and logging out asks the password for it', (tester) async {
    _phone(tester);
    final (HomeServer server, FakeServerPorts ports, FakeBook book) = _server();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
    await tester.pumpWidget(themed(AccountScreen(server: server, phoneKey: FakeClientKey())));
    await settle(tester);

    expect(find.text('Owner'), findsOneWidget);
    expect(find.textContaining('keeps its own keys on its feeders'), findsOneWidget);
    await tester.tap(find.text('Log out').first);
    await settle(tester);
    expect(find.textContaining('The phones it invited keep theirs too'), findsOneWidget);
    await tester.enterText(find.widgetWithText(TextField, 'Password'), 'hunter2');
    await tester.pump();
    await tester.tap(find.widgetWithText(TextButton, 'Log out'));
    await settle(tester);

    expect(ports.session.logoutPasswords, ['hunter2']);
    expect(book.server, isNull);
    expect(book.feeders, [_kitchen]);
  });

  test('pets and foods are read, and a change reads them again', () async {
    final session = FakeServerSession();
    session.home.pets = const [_cat];
    final flow = HouseholdFlow(session);
    addTearDown(flow.dispose);
    await flow.load();
    expect(flow.state.pets, [_cat]);

    expect(await flow.saveFood(brand: 'Acme', name: 'Chicken', kcalPerKg: 3800, bagGrams: 2000), isTrue);
    expect(flow.state.foods.single.name, 'Chicken');

    session.home.fails = serverError(ServerErrorKind.foodInUse);
    expect(await flow.deleteFood('f1'), isFalse);
    expect(flow.state.problem?.error.kind, ServerErrorKind.foodInUse);
    expect(flow.state.problem?.mayHaveRun, isFalse);
  });

  test('a new pet whose answer was lost is read back and said to be unsure, never added again', () async {
    final session = FakeServerSession();
    final flow = HouseholdFlow(session);
    addTearDown(flow.dispose);
    await flow.load();

    session.home.lost = serverError(ServerErrorKind.interrupted, mayHaveRun: true);
    expect(await flow.addPet(name: 'Tom', species: Species.cat), isFalse);
    expect(flow.state.pets.map((pet) => pet.name), ['Tom'], reason: 'the read after it shows it took');
    expect(flow.state.problem?.mayHaveRun, isTrue);
    expect(flow.state.busy, isFalse);
    expect(session.home.changes, ['create pet Tom']);

    // A server that said it failed partway reads the same way.
    session.home.lost = serverError(ServerErrorKind.serverFailed, mayHaveRun: true);
    expect(await flow.saveFood(brand: 'Acme', name: 'Fish', kcalPerKg: 3500, bagGrams: 2000), isFalse);
    expect(flow.state.foods.map((food) => food.name), ['Fish']);
    expect(flow.state.problem?.mayHaveRun, isTrue);

    // So does a page no Wellfed server sends, as a proxy's after the insert
    // landed: the bridge says it may have run, whatever its words.
    session.home.lost = serverError(ServerErrorKind.protocol, mayHaveRun: true);
    expect(await flow.addPet(name: 'Kit', species: Species.cat), isFalse);
    expect(flow.state.pets.map((pet) => pet.name), ['Tom', 'Kit']);
    final RecordProblem? problem = flow.state.problem;
    expect(problem?.mayHaveRun, isTrue);
    expect(problem == null ? null : recordProblemWords(problem), contains('may or may not have taken'));
    expect(session.home.changes, ['create pet Tom', 'create food Fish', 'create pet Kit']);
  });

  testWidgets('a pet edit shows the new pet, and a refused one says why', (tester) async {
    _phone(tester);
    final session = FakeServerSession();
    session.home.pets = const [_cat];
    await tester.pumpWidget(themed(PetScreen(flow: PetFlow(session, _cat), feeders: const [])));
    await settle(tester);

    session.home.fails = serverError(ServerErrorKind.invalid);
    await tester.tap(find.text('Edit'));
    await settle(tester);
    await tester.tap(find.text('Save'));
    await settle(tester);
    expect(find.textContaining('The server did not take that'), findsOneWidget);

    session.home.fails = null;
    await tester.tap(find.text('Edit'));
    await settle(tester);
    await tester.enterText(find.widgetWithText(TextField, 'Miso'), 'Tom');
    await tester.pump();
    await tester.tap(find.text('Save'));
    await settle(tester);
    expect(find.text('Tom'), findsOneWidget);
    expect(find.textContaining('The server did not take that'), findsNothing);
  });

  testWidgets('removing a pet closes its screen, and a refused removal stays and says why', (tester) async {
    _phone(tester);
    final session = FakeServerSession();
    session.home.pets = const [_cat];
    await tester.pumpWidget(
      themed(
        Builder(
          builder: (context) => TextButton(
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => PetScreen(flow: PetFlow(session, _cat), feeders: const []),
              ),
            ),
            child: const Text('Open'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open'));
    await settle(tester);

    Future<void> remove() async {
      await tester.tap(find.text('Remove'));
      await settle(tester);
      await tester.tap(find.widgetWithText(TextButton, 'Remove').last);
      await settle(tester);
    }

    session.home.fails = serverError(ServerErrorKind.forbidden);
    await remove();
    expect(find.text('Your account may not do that.'), findsOneWidget);

    session.home.fails = null;
    await remove();
    expect(find.text('Open'), findsOneWidget);
    expect(session.home.pets, isEmpty);
  });

  testWidgets('an uncalibrated feeder counts portions: no grams and no food left', (tester) async {
    _phone(tester);
    final session = FakeServerSession();
    session.home.food = const ServerFeederFood(
      weight: PortionWeight.uncalibrated(),
      sinceRefill: ServerSinceRefill(portions: 14, uncertainPortions: 1),
      rate: FeedRate(portions: 6, hours: 24),
      anomalies: [],
    );
    await tester.pumpWidget(themed(FeederFoodScreen(flow: FeederFoodFlow(session, 'id-k'), name: 'Kitchen')));
    await settle(tester);

    expect(find.textContaining('14 portions'), findsOneWidget);
    expect(find.text('6 portions in 24 hours'), findsOneWidget);
    expect(find.textContaining(' g left'), findsNothing);
    expect(find.textContaining('a day'), findsNothing);

    await tester.tap(find.text('Full bag'));
    await settle(tester);
    expect(session.home.changes, ['refill id-k full']);
  });

  testWidgets('a calibrated feeder shows what is left', (tester) async {
    _phone(tester);
    final session = FakeServerSession();
    session.home.food = const ServerFeederFood(
      weight: PortionWeight.calibration(portionMg: 9000),
      sinceRefill: ServerSinceRefill(
        portions: 14,
        uncertainPortions: 0,
        left: ServerFoodLeft(gramsUsed: 126, gramsLeft: 874, gramsLeftLeast: 874),
      ),
      rate: FeedRate(portions: 6, hours: 24, gramsPerDay: 54),
      anomalies: [],
    );
    await tester.pumpWidget(themed(FeederFoodScreen(flow: FeederFoodFlow(session, 'id-k'), name: 'Kitchen')));
    await settle(tester);

    expect(find.textContaining('874 g left'), findsOneWidget);
    expect(find.text('54 g a day'), findsOneWidget);
  });

  testWidgets("a pet's day from uncalibrated feeders counts portions, with no calories", (tester) async {
    _phone(tester);
    final session = FakeServerSession();
    session.home.days = const [
      ServerIntakeDay(
        date: '2026-11-04',
        estimate: false,
        unweighed: [ServerFeederPortions(feeder: 'id-k', portions: 3, uncertainPortions: 0)],
      ),
    ];
    await tester.pumpWidget(
      themed(
        PetScreen(
          flow: PetFlow(session, _cat),
          feeders: const [ListedFeeder(id: 'id-k', identity: 'aa', name: 'Kitchen')],
        ),
      ),
    );
    await settle(tester);

    expect(find.text('3 portions from Kitchen'), findsOneWidget);
    expect(find.textContaining('kcal'), findsNothing);
  });

  test('the long history reads every page and shows the newest first', () async {
    final session = FakeServerSession();
    session.home.pages = {
      null: ServerEventPage(
        entries: [
          for (final int seq in [1, 2]) ServerEntry(epoch: 1, receivedMs: 0, entry: _entry(seq)),
        ],
        next: BigInt.two,
      ),
      BigInt.two: ServerEventPage(entries: [ServerEntry(epoch: 1, receivedMs: 0, entry: _entry(3))]),
    };
    final flow = ServerHistoryFlow(session, 'id-k');
    addTearDown(flow.dispose);
    await flow.load();

    expect(flow.state.reading, isFalse);
    expect(flow.state.entries.map((entry) => entry.seq.toInt()), [3, 2, 1]);
  });

  test('an adoption whose answer was lost is only checked again, never sent again', () async {
    final (HomeServer server, _, _) = _server();
    final ports = FakeFeederPorts()..adoptAnswer = Adoption.unsettled(error: serverError(ServerErrorKind.interrupted));
    final flow = AdoptFlow(ports, server);
    addTearDown(flow.dispose);

    await flow.adopt(_kitchen);
    expect(flow.state['aa'], isA<AdoptUnsettled>());
    await flow.adopt(_kitchen);
    expect(ports.adoptSteps, ['adopt aa', 'check aa']);
  });
}
