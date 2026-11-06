import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/server/server.dart';

import '../support/fakes.dart';

const _kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');

(HomeServer, FakeServerPorts, FakeRouter, FakeBook) _server() {
  final book = FakeBook()..feeders = [_kitchen];
  final ports = FakeServerPorts();
  final router = FakeRouter();
  final server = HomeServer(book, ports, router);
  addTearDown(server.dispose);
  return (server, ports, router, book);
}

void main() {
  test('no server configured runs nothing', () {
    final (HomeServer server, FakeServerPorts ports, FakeRouter router, _) = _server();
    expect(server.state, isA<ServerAbsent>());
    expect(ports.logins, isEmpty);
    expect(router.connected, isEmpty);
  });

  test('a login keeps the token in secure storage and the password nowhere', () async {
    final (HomeServer server, FakeServerPorts ports, FakeRouter router, FakeBook book) = _server();
    final LoginOutcome outcome = await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');

    expect(outcome, isA<LoginDone>());
    expect(ports.logins.single.phone, 'Pixel', reason: 'the token is named for this phone');
    expect(ports.kept, 'token-secret');
    expect(book.server?.link, serverLink);
    expect('${book.server?.link.url}${book.server?.link.user}${book.server?.link.tokenId}', isNot(contains('hunter2')));
    expect(router.connected, [serverOwner]);
    expect(server.state, isA<ServerLinked>().having((linked) => linked.account, 'account', serverOwner));
  });

  test('a refused login says why and keeps nothing', () async {
    final (HomeServer server, FakeServerPorts ports, _, FakeBook book) = _server();
    ports.loginFails = serverError(ServerErrorKind.badCredentials);
    final LoginOutcome outcome = await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'wrong');

    expect(outcome, isA<LoginFailed>().having((failed) => failed.error.kind, 'kind', ServerErrorKind.badCredentials));
    expect(ports.kept, isNull);
    expect(book.server, isNull);
    expect(server.state, isA<ServerAbsent>());
  });

  test('logging out revokes the token and forgets the server, and the feeders stay', () async {
    final (HomeServer server, FakeServerPorts ports, FakeRouter router, FakeBook book) = _server();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');

    expect(await server.logOut(), isTrue);
    expect(ports.session.logouts, 1);
    expect(ports.kept, isNull);
    expect(book.server, isNull);
    expect(book.feeders, [_kitchen]);
    expect(router.disconnects, greaterThan(0));
    expect(server.state, isA<ServerAbsent>());
  });

  test('a logout the server cannot take still forgets it, and says so', () async {
    final (HomeServer server, FakeServerPorts ports, _, FakeBook book) = _server();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
    ports.session.logoutFails = serverError(ServerErrorKind.unreachable);

    expect(await server.logOut(), isFalse);
    expect(book.server, isNull);
  });

  test('removing the server leaves its token as it is', () async {
    final (HomeServer server, FakeServerPorts ports, _, FakeBook book) = _server();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');

    await server.removeServer();
    expect(ports.session.logouts, 0);
    expect(ports.forgets, 1);
    expect(book.server, isNull);
  });

  test('a token the server refuses is signed out, and nothing tries it again', () async {
    final (HomeServer server, FakeServerPorts ports, FakeRouter router, _) = _server();
    ports.session.accountFails = serverError(ServerErrorKind.tokenRevoked);
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');

    expect(server.state, isA<ServerLinked>().having((linked) => linked.problem, 'problem', ServerProblem.signedOut));
    expect(router.connected, isEmpty);
  });

  test("a kept server whose token is gone keeps the phone's own keys closed", () {
    final book = FakeBook()
      ..feeders = [_kitchen]
      ..server = const KnownServer(link: serverLink, feeders: []);
    final router = FakeRouter();
    final server = HomeServer(book, FakeServerPorts(), router);
    addTearDown(server.dispose);
    return pumpEventQueue().then((_) {
      expect(router.refusals, 1);
      expect(router.connected, isEmpty);
      expect(server.state, isA<ServerLinked>().having((linked) => linked.problem, 'problem', ServerProblem.signedOut));
    });
  });

  test('a cold start hands the router the kept account, and keeps the one the server reads', () async {
    const viewer = ServerAccount(id: 'u2', name: 'sam', role: ServerRole.viewer);
    final book = FakeBook()
      ..feeders = [_kitchen]
      ..server = const KnownServer(link: serverLink, feeders: [], account: viewer);
    final ports = FakeServerPorts()..kept = 'token-secret';
    final router = FakeRouter();
    final server = HomeServer(book, ports, router);
    addTearDown(server.dispose);
    await pumpEventQueue();

    expect(router.resumed, [viewer]);
    expect(book.server?.account, serverOwner, reason: 'what the server says now is kept');
  });

  test('a kept server reaches the router before anything is awaited', () {
    const viewer = ServerAccount(id: 'u2', name: 'sam', role: ServerRole.viewer);
    const kept = KnownServer(link: serverLink, feeders: [], account: viewer);
    final book = FakeBook()
      ..feeders = [_kitchen]
      ..server = kept;
    final router = FakeRouter();
    final server = HomeServer(book, FakeServerPorts()..kept = 'token-secret', router);
    addTearDown(server.dispose);

    // Synchronously, in the constructor: no session or feed can come first.
    expect(router.configured, [kept]);
    expect(router.log, ['configure']);
    expect(router.resumed, isEmpty, reason: 'the secret is not read yet');
  });

  test('an account the router reads again is kept for the next cold start', () async {
    final (HomeServer server, _, FakeRouter router, FakeBook book) = _server();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');

    const demoted = ServerAccount(id: 'u1', name: 'ada', role: ServerRole.viewer);
    router.said.add(const RouterUpdate.account(account: demoted));
    await pumpEventQueue();

    expect(server.state, isA<ServerLinked>().having((linked) => linked.account, 'account', demoted));
    expect(book.server?.account, demoted);
  });

  test("a login keeps the phone's own keys closed until the server says who it is", () async {
    final (HomeServer server, FakeServerPorts ports, FakeRouter router, _) = _server();
    final answer = Completer<void>();
    ports.session.accountWait = answer.future;
    final Future<LoginOutcome> login = server.logIn(url: 'http://home.lan:8080', name: 'sam', password: 'hunter2');
    await pumpEventQueue();

    // The last server let go, then this one told with no account: no key of
    // the phone's own while the account read is out.
    expect(router.log, ['disconnect', 'configure', 'resume']);
    expect(router.configured.single.account, isNull);
    expect(router.configured.single.link, serverLink);

    answer.complete();
    await login;
    expect(router.connected, [serverOwner]);
  });

  test("a login's file never keeps the last server's copy over the new one", () async {
    final (HomeServer server, FakeServerPorts ports, _, FakeBook book) = _server();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');

    // A copy of the first server is still being written as the next login goes.
    final slow = Completer<void>();
    book.serverWrite = slow.future;
    final Future<void> refreshing = server.refresh();
    await pumpEventQueue();
    const other = ServerLink(
      url: 'http://other.lan:8080',
      user: 'sam',
      tokenId: '00000000-0000-0000-0000-000000000009',
    );
    // The new server answers nothing yet, so only the login's own write counts.
    final answer = Completer<void>();
    ports.session = FakeServerSession()
      ..linked = other
      ..accountWait = answer.future;
    final Future<LoginOutcome> login = server.logIn(url: 'http://other.lan:8080', name: 'sam', password: 'pw');
    await pumpEventQueue();
    slow.complete();
    await refreshing;
    await pumpEventQueue();

    expect(book.server?.link, other);
    expect(book.server?.account, isNull, reason: 'no account of the last server goes with it');
    answer.complete();
    await login;
  });
}
