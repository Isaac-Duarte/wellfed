import 'dart:async';

import 'package:wellfed/platform/server_ports.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';

import 'fakes.dart';

const serverLink = ServerLink(
  url: 'http://home.lan:8080',
  user: 'ada',
  tokenId: '00000000-0000-0000-0000-000000000001',
);
const serverOwner = ServerAccount(id: 'u1', name: 'ada', role: ServerRole.owner);

ServerError serverError(ServerErrorKind kind) => ServerError(kind: kind, message: kind.name);

/// A session with the owner's server the test answers. Only what the app's
/// controllers call is here; anything else fails the test.
class FakeServerSession with FakeOpaque implements ServerSession {
  FakeServerSession({this.who = serverOwner});

  /// Who the token speaks for; a failure when set to one.
  ServerAccount who;
  ServerError? accountFails;
  ServerError? logoutFails;
  int logouts = 0;
  int closes = 0;

  /// Each feeder's status, by its server id.
  final details = <String, ServerFeederDetail>{};

  @override
  ServerLink link() => serverLink;

  @override
  String secret() => 'token-secret';

  @override
  Future<ServerAccount> account() async {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (accountFails case final error?) throw error;
    return who;
  }

  @override
  Future<void> logout() async {
    logouts++;
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (logoutFails case final error?) throw error;
  }

  @override
  Future<ServerFeederDetail> feeder({required String id}) async {
    final ServerFeederDetail? detail = details[id];
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (detail == null) throw serverError(ServerErrorKind.notFound);
    return detail;
  }

  @override
  void close() => closes++;

  // The controllers call none of the rest.
  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError('${invocation.memberName}');
}

/// Secure storage and the server's login, in memory.
class FakeServerPorts implements ServerPorts {
  FakeServerSession session = FakeServerSession();

  /// What the next login throws.
  ServerError? loginFails;

  /// Every login's arguments, the password among them, as the phone sent them.
  final logins = <({String url, String name, String password, String phone})>[];
  String? kept;
  int forgets = 0;

  @override
  Future<ServerSession> login({
    required String url,
    required String name,
    required String password,
    required String phone,
  }) async {
    logins.add((url: url, name: name, password: password, phone: phone));
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (loginFails case final error?) throw error;
    return session;
  }

  @override
  ServerSession restore(ServerLink link, String secret) => session;

  @override
  Future<String?> secret() async => kept;

  @override
  Future<void> keepSecret(String secret) async => kept = secret;

  @override
  Future<void> forgetSecret() async {
    kept = null;
    forgets++;
  }
}
