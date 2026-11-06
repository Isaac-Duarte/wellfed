import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/server/server.dart';

import '../support/fakes.dart';

const _kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');
const _porch = KnownFeeder(identity: 'bb', name: 'Porch', address: 'lan:4748');

ServerFeeder _listed(String id, String identity, String name, {ServerConnection link = ServerConnection.online}) =>
    ServerFeeder(id: id, name: name, identity: identity, role: ClientRole.owner, connection: link);

/// The phone's book, its ports, a server that lists `listed`, and the fleet.
Future<(FeederFleet, FakeFeederPorts, FakeServerPorts, FakeBook)> _fleet(
  List<ServerFeeder> listed, {
  KnownServer? kept,
  void Function(FakeServerPorts)? setUp,
}) async {
  final book = FakeBook()
    ..feeders = [_kitchen, _porch]
    ..server = kept;
  final ports = FakeFeederPorts();
  ports.fakeRouter.listed = listed;
  final serverPorts = FakeServerPorts()..kept = kept == null ? null : 'token-secret';
  setUp?.call(serverPorts);
  final server = HomeServer(book, serverPorts, ports.router);
  final fleet = FeederFleet(book, ports, server: server);
  addTearDown(() {
    fleet.dispose();
    server.dispose();
  });
  if (kept == null) {
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
  }
  await pumpEventQueue();
  return (fleet, ports, serverPorts, book);
}

void main() {
  test('with no server the fleet is the phone own feeders and offers nothing to adopt', () async {
    final book = FakeBook()..feeders = [_kitchen, _porch];
    final ports = FakeFeederPorts();
    final fleet = FeederFleet(book, ports);
    addTearDown(fleet.dispose);
    await pumpEventQueue();

    expect(fleet.all.map((feeder) => feeder.state.known.identity), ['aa', 'bb']);
    expect(fleet.adoptable, isEmpty);
    expect(fleet.all.every((feeder) => feeder.ownKey && feeder.state.serverId == null), isTrue);
  });

  test("the server's list is the fleet, its own key used where the phone holds one", () async {
    final (FeederFleet fleet, FakeFeederPorts ports, _, _) = await _fleet([
      _listed('id-k', 'aa', 'Kitchen'),
      _listed('id-h', 'cc', 'Hall'),
    ]);

    expect(fleet.all.map((feeder) => feeder.state.known.identity), unorderedEquals(['aa', 'bb', 'cc']));
    final FeederController? hall = fleet['cc'];
    expect(hall?.ownKey, isFalse);
    expect(hall?.state.known.name, 'Hall');
    expect(fleet['aa']?.state.serverId, 'id-k');
    // No session for a feeder the phone holds no key on.
    expect(ports.sessions.map((session) => session.identity), isNot(contains('cc')));
    // A feeder the phone paired that the server does not list: offered.
    expect(fleet.adoptable, [_porch]);
  });

  test('the server list is kept for a cold start, and a cold start with the server down still goes direct', () async {
    final (FeederFleet _, FakeFeederPorts ports, _, FakeBook book) = await _fleet([_listed('id-k', 'aa', 'Kitchen')]);
    expect(book.server?.feeders, [const ListedFeeder(id: 'id-k', identity: 'aa', name: 'Kitchen')]);

    final (FeederFleet cold, FakeFeederPorts coldPorts, _, _) = await _fleet(
      const [],
      kept: book.server,
      setUp: (server) => server.session.accountFails = serverError(ServerErrorKind.unreachable),
    );
    expect(coldPorts.fakeRouter.configured, [book.server]);
    expect(coldPorts.fakeRouter.log.first, 'configure', reason: 'before any session or feed');
    expect(coldPorts.fakeRouter.connected, isEmpty, reason: 'the server never answered');
    expect(cold['aa']?.ownKey, isTrue);
    expect(coldPorts.sessions.map((session) => session.identity), contains('aa'));
    expect(ports.fakeRouter.connected, [serverOwner]);
  });

  test("the server's view stands in while the phone has no session of its own", () async {
    final (FeederFleet fleet, _, FakeServerPorts server, _) = await _fleet(
      [_listed('id-h', 'cc', 'Hall')],
      setUp: (server) => server.session.details['id-h'] = ServerFeederDetail(
        feeder: _listed('id-h', 'cc', 'Hall'),
        status: ServerSnapshot(atMs: 1000, live: true, card: fakeCard()),
      ),
    );
    await pumpEventQueue();

    final FeederState? hall = fleet['cc']?.state;
    expect(hall?.connection, Connection.online);
    expect(hall?.cardIsCurrent, isTrue);
    expect(hall?.role, ClientRole.owner);
    expect(hall?.canFeed, isTrue);
    expect(server.session.logouts, 0);
  });

  test('a feeder the server reaches on another build reads as needing an update, and feeds no one', () async {
    final ServerFeeder hall = _listed('id-h', 'cc', 'Hall', link: ServerConnection.otherBuild);
    final (FeederFleet fleet, _, _, _) = await _fleet([
      hall,
    ], setUp: (server) => server.session.details['id-h'] = ServerFeederDetail(feeder: hall));
    await pumpEventQueue();

    final FeederState? state = fleet['cc']?.state;
    expect(state?.connection, Connection.otherBuild);
    expect(state?.cardIsCurrent, isFalse);
    expect(state?.canFeed, isFalse);
  });
}
