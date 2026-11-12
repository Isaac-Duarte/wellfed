import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/server/log_out.dart';
import 'package:wellfed/state/server/server.dart';

import '../support/fakes.dart';

const _kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');

(HomeServer, FakeServerPorts, FakeRouter, FakeBook) _server({ClientKey? key}) {
  final book = FakeBook()..feeders = [_kitchen];
  final ports = FakeServerPorts();
  final router = FakeRouter();
  final server = HomeServer(book, ports, router, key ?? FakeClientKey());
  addTearDown(server.dispose);
  return (server, ports, router, book);
}

void main() {
  final key = FakeClientKey();

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

  test("each list of the feeders has the router claim the phone's own key, and a failed one does not", () async {
    final (HomeServer server, FakeServerPorts ports, FakeRouter router, _) = _server(key: key);
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
    expect(router.claims, [same(key)], reason: 'claimed once the login lists the feeders');

    await server.refresh();
    expect(router.claims, hasLength(2), reason: 'each list tries again');

    router.refreshWait = Completer<List<ServerFeeder>>()..completeError(serverError(ServerErrorKind.unreachable));
    await server.refresh();
    expect(router.claims, hasLength(2), reason: 'a list that failed claims nothing');
  });

  test('a login whose token storage cannot keep revokes it and keeps nothing', () async {
    final (HomeServer server, FakeServerPorts ports, _, FakeBook book) = _server();
    ports.keepFails = PlatformException(code: 'keystore');
    final LoginOutcome outcome = await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');

    expect(outcome, isA<LoginNotKept>());
    expect(ports.session.logouts, 1, reason: 'a token kept nowhere is revoked');
    expect(book.server, isNull);
    expect(server.state, isA<ServerAbsent>());
  });

  test('a server removed while storage cannot forget its token is still gone from the file', () async {
    final (HomeServer server, FakeServerPorts ports, _, FakeBook book) = _server();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
    ports.forgetFails = PlatformException(code: 'keystore');

    await server.removeServer();
    expect(book.server, isNull);
    expect(server.state, isA<ServerAbsent>());
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

  test('logging out revokes the token with the password and forgets the server, and the feeders stay', () async {
    final (HomeServer server, FakeServerPorts ports, FakeRouter router, FakeBook book) = _server();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');

    expect(await server.logOut('hunter2', key), isA<LoggedOut>().having((out) => out.revoked, 'revoked', isTrue));
    expect(ports.session.logoutPasswords, ['hunter2']);
    expect(ports.session.logoutKeys, [key], reason: 'the phone names its own key');
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

    expect(await server.logOut('hunter2', key), isA<LoggedOut>().having((out) => out.revoked, 'revoked', isFalse));
    expect(book.server, isNull);
  });

  test('a wrong password logs nothing out', () async {
    final (HomeServer server, FakeServerPorts ports, _, FakeBook book) = _server();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
    ports.session.logoutFails = serverError(ServerErrorKind.badCredentials);

    final LogOutOutcome outcome = await server.logOut('wrong', key);
    expect(outcome, isA<LogOutRefused>().having((out) => out.error.kind, 'kind', ServerErrorKind.badCredentials));
    expect(book.server, isNotNull);
    expect(server.state, isA<ServerLinked>());
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

  testWidgets('a server that proves another key is not tried again', (tester) async {
    final (HomeServer server, FakeServerPorts ports, FakeRouter router, _) = _server();
    ports.session.accountFails = serverError(ServerErrorKind.keyChanged);
    await server.logIn(url: 'https://192.168.1.20:8443', name: 'ada', password: 'hunter2');
    expect(server.state, isA<ServerLinked>().having((linked) => linked.problem, 'problem', ServerProblem.keyChanged));

    ports.session.accountFails = null;
    await tester.pump(HomeServer.retryWait * 2);
    expect(server.state, isA<ServerLinked>().having((linked) => linked.problem, 'problem', ServerProblem.keyChanged));
    expect(router.connected, isEmpty);
  });

  test("logging in again checks the kept server's key, whatever the network announces", () async {
    final (HomeServer server, FakeServerPorts ports, _, _) = _server();
    ports.session.accountFails = serverError(ServerErrorKind.tokenRevoked);
    await server.logIn(url: 'https://192.168.1.20:8443', name: 'ada', password: 'hunter2', key: 'announced');
    expect(ports.logins.last.key, 'announced');

    await server.logIn(url: 'https://192.168.1.20:8443', name: 'ada', password: 'hunter2', key: 'another');
    expect(ports.logins.last.key, serverLink.key, reason: 'the kept server stays pinned');
  });

  test("a kept server whose token is gone keeps the phone's own keys closed", () {
    final book = FakeBook()
      ..feeders = [_kitchen]
      ..server = const KnownServer(link: serverLink, feeders: []);
    final router = FakeRouter();
    final server = HomeServer(book, FakeServerPorts(), router, FakeClientKey());
    addTearDown(server.dispose);
    return pumpEventQueue().then((_) {
      expect(router.refusals, 1);
      expect(router.connected, isEmpty);
      expect(server.state, isA<ServerLinked>().having((linked) => linked.problem, 'problem', ServerProblem.signedOut));
    });
  });

  test("a token secure storage can't read keeps the phone's keys closed, and asks no login over it", () async {
    final book = FakeBook()
      ..feeders = [_kitchen]
      ..server = const KnownServer(link: serverLink, feeders: []);
    final ports = FakeServerPorts()
      ..kept = 'token-secret'
      ..secretFails = Exception('keystore');
    final router = FakeRouter();
    final server = HomeServer(book, ports, router, FakeClientKey());
    addTearDown(server.dispose);
    await pumpEventQueue();

    expect(router.refusals, 1);
    expect(router.connected, isEmpty);
    expect(server.state, isA<ServerLinked>().having((linked) => linked.problem, 'problem', ServerProblem.storage));
    expect(ports.kept, 'token-secret', reason: 'the token stays for the next start');
    expect(ports.forgets, 0);
    expect(book.server, isNotNull);
  });

  test('a kept server at plain http beyond the home network is not used, and says why', () async {
    final book = FakeBook()
      ..feeders = [_kitchen]
      ..server = const KnownServer(link: serverLink, feeders: []);
    final ports = FakeServerPorts()
      ..kept = 'token-secret'
      ..restoreFails = serverError(ServerErrorKind.plainHttp);
    final router = FakeRouter();
    final server = HomeServer(book, ports, router, FakeClientKey());
    addTearDown(server.dispose);
    await pumpEventQueue();

    expect(router.refusals, 1);
    expect(router.connected, isEmpty);
    expect(server.state, isA<ServerLinked>().having((linked) => linked.problem, 'problem', ServerProblem.plainHttp));
  });

  testWidgets('a refresh of a session a new login replaced neither shows a problem nor retries', (tester) async {
    final (HomeServer server, FakeServerPorts ports, FakeRouter router, _) = _server();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
    final stale = Completer<List<ServerFeeder>>();
    router.refreshWait = stale;
    final Future<void> refreshing = server.refresh();

    ports.session = FakeServerSession();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
    final int refreshes = router.refreshes;
    stale.completeError(serverError(ServerErrorKind.closed));
    await refreshing;

    expect(server.state, isA<ServerLinked>().having((linked) => linked.problem, 'problem', isNull));
    await tester.pump(HomeServer.retryWait + const Duration(seconds: 1));
    expect(router.refreshes, refreshes, reason: 'no retry on the new session');
  });

  testWidgets('an account read of a session a new login replaced neither shows a problem nor retries', (tester) async {
    final (HomeServer server, FakeServerPorts ports, FakeRouter router, _) = _server();
    final answer = Completer<void>();
    ports.session
      ..accountWait = answer.future
      ..accountFails = serverError(ServerErrorKind.closed);
    final Future<LoginOutcome> first = server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
    await tester.pump();

    ports.session = FakeServerSession();
    await server.logIn(url: 'http://home.lan:8080', name: 'ada', password: 'hunter2');
    final int refreshes = router.refreshes;
    answer.complete();
    await first;

    expect(server.state, isA<ServerLinked>().having((linked) => linked.problem, 'problem', isNull));
    await tester.pump(HomeServer.retryWait + const Duration(seconds: 1));
    expect(router.refreshes, refreshes, reason: 'no retry on the new session');
  });

  test('a cold start hands the router the kept account, and keeps the one the server reads', () async {
    const viewer = ServerAccount(id: 'u2', name: 'sam', role: ServerRole.viewer);
    final book = FakeBook()
      ..feeders = [_kitchen]
      ..server = const KnownServer(link: serverLink, feeders: [], account: viewer);
    final ports = FakeServerPorts()..kept = 'token-secret';
    final router = FakeRouter();
    final server = HomeServer(book, ports, router, FakeClientKey());
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
    final server = HomeServer(book, FakeServerPorts()..kept = 'token-secret', router, FakeClientKey());
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
      url: 'https://192.168.1.30:8443',
      key: 'cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd',
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
