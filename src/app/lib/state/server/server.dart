import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/server_ports.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/fleet/app_state.dart';

/// Why the owner's server is not serving this phone right now.
enum ServerProblem {
  /// Nothing answered at its address: the phone goes to its feeders directly.
  unreachable,

  /// The server no longer takes this phone's token (revoked, expired or the
  /// account deleted), or the account is disabled. The phone's own keys stop
  /// too, until the owner logs in again or removes the server.
  signedOut,

  /// It answered something this app does not read.
  other,
}

/// The owner's server as this phone knows it.
sealed class ServerState {
  const ServerState();
}

/// No server: the app works with its own feeders, as it always has.
final class ServerAbsent extends ServerState {
  /// The state with no server.
  const ServerAbsent();
}

/// Logged in to a server.
final class ServerLinked extends ServerState {
  /// A server at `link`.
  const ServerLinked({required this.link, required this.feeders, this.account, this.listed = false, this.problem});

  /// Where it is, and as whom.
  final ServerLink link;

  /// Who the phone's token speaks for, once the server has said.
  final ServerAccount? account;

  /// The feeders it lists: the last list it gave, kept on the phone, until it
  /// gives a new one.
  final List<ListedFeeder> feeders;

  /// [feeders] is what the server said since the app started, not only the
  /// kept copy.
  final bool listed;

  /// What stops it serving the phone now; `null` while it does.
  final ServerProblem? problem;

  /// The same server with the named parts changed.
  ServerLinked copyWith({
    ServerAccount? account,
    List<ListedFeeder>? feeders,
    bool? listed,
    ServerProblem? Function()? problem,
  }) => ServerLinked(
    link: link,
    account: account ?? this.account,
    feeders: feeders == null ? this.feeders : List.unmodifiable(feeders),
    listed: listed ?? this.listed,
    problem: problem == null ? this.problem : problem(),
  );
}

/// How a login ended.
sealed class LoginOutcome {
  const LoginOutcome();
}

/// Logged in: the phone's token is kept, and the password is not.
final class LoginDone extends LoginOutcome {
  /// A login that went through.
  const LoginDone();
}

/// The server refused or could not be reached; `error` says which.
final class LoginFailed extends LoginOutcome {
  /// A login that did not go through.
  const LoginFailed(this.error);

  /// What the server, or the way to it, said.
  final ServerError error;
}

/// The owner's server: the login, the phone's token, the feeders it lists
/// and its live stream, which goes through the path router so the router
/// settles pinned writes by what the server says. The app with no server is
/// the app as it was: nothing here runs until a login.
class HomeServer extends ChangeNotifier {
  /// The server `_book` keeps, if any, served at once.
  HomeServer(this._book, this._ports, this._router) {
    final KnownServer? kept = _book.server;
    if (kept != null) {
      _state = ServerLinked(link: kept.link, feeders: kept.feeders);
      unawaited(_restore(kept));
    }
  }

  /// A server that went quiet is tried again after this: the path router
  /// passes over a down server for as long before it tries it again.
  static const retryWait = Duration(seconds: 30);

  final FeederBook _book;
  final ServerPorts _ports;
  final PathRouter? _router;

  /// The server as it stands.
  ServerState get state => _state;
  ServerState _state = const ServerAbsent();

  ServerSession? _session;
  StreamSubscription<ServerUpdate>? _following;
  String? _resume;
  Timer? _again;
  bool _disposed = false;

  final _events = StreamController<ServerEvent>.broadcast();

  /// The feeders' events the server's stream brings, for the fleet: history
  /// entries and the server's links to them.
  Stream<ServerEvent> get events => _events.stream;

  /// What this account may do on its feeders, as a feeder's own role names
  /// it: `null` for a viewer, and before the server has said.
  ClientRole? get feederRole => switch (_state) {
    ServerLinked(account: ServerAccount(role: ServerRole.owner)) => ClientRole.owner,
    ServerLinked(account: ServerAccount(role: ServerRole.member)) => ClientRole.member,
    _ => null,
  };

  /// Log in to the server at `url` as `name`. The password is used once, to
  /// mint this phone's own token, and never kept.
  Future<LoginOutcome> logIn({required String url, required String name, required String password}) async {
    final ServerSession session;
    try {
      session = await _ports.login(url: url, name: name, password: password, phone: _phone);
    } on ServerError catch (error) {
      return LoginFailed(error);
    }
    if (_disposed) return const LoginDone();

    await _drop();
    await _ports.keepSecret(session.secret());
    final ServerLink link = session.link();
    await _book.setServer(KnownServer(link: link, feeders: const []));
    _session = session;
    _set(ServerLinked(link: link, feeders: const []));
    await _connect(session);
    return const LoginDone();
  }

  /// Revoke this phone's token on the server, then forget the server. The
  /// phone keeps its own keys on its feeders and reaches them directly.
  /// Whether the server took the revocation: when it could not be reached the
  /// token stays valid until it is revoked there.
  Future<bool> logOut() async {
    final ServerSession? session = _session;
    var revoked = false;
    if (session != null) {
      try {
        await session.logout();
        revoked = true;
      } on ServerError {
        revoked = false;
      }
    }
    await _forget();
    return revoked;
  }

  /// Forget the server on this phone alone, its token left as it is.
  Future<void> removeServer() => _forget();

  /// Read the server's feeders again.
  Future<void> refresh() async {
    final PathRouter? router = _router;
    if (router == null || _session == null) return;
    final List<ServerFeeder> feeders;
    try {
      feeders = await router.refresh();
    } on ServerError catch (error) {
      _failed(error);
      return;
    }
    if (_disposed) return;

    final List<ListedFeeder> listed = [
      for (final ServerFeeder feeder in feeders)
        if (feeder.archivedMs == null) ListedFeeder(id: feeder.id, identity: feeder.identity, name: feeder.name),
    ];
    if (_state case final ServerLinked linked) {
      _set(linked.copyWith(feeders: listed, listed: true, problem: () => null));
      try {
        await _book.setServer(KnownServer(link: linked.link, feeders: listed));
      } on Object catch (error) {
        // Only the copy for a cold start; the next list keeps it.
        debugPrint("the server's feeder list was not kept: $error");
      }
    }
  }

  /// One feeder's status and link as the server holds them; `null` when it
  /// could not say.
  Future<ServerFeederDetail?> detail(String id) async {
    final ServerSession? session = _session;
    if (session == null) return null;
    try {
      return await session.feeder(id: id);
    } on ServerError {
      return null;
    }
  }

  /// The app is back in the foreground: a server that was away is tried now.
  void resumed() {
    if (_state case ServerLinked(problem: ServerProblem.unreachable)) _retryNow();
  }

  String get _phone => _book.label.isEmpty ? 'Wellfed phone' : _book.label;

  Future<void> _restore(KnownServer kept) async {
    final String? secret = await _ports.secret();
    if (_disposed) return;
    if (secret == null) {
      _set(ServerLinked(link: kept.link, feeders: kept.feeders, problem: ServerProblem.signedOut));
      return;
    }
    final ServerSession session;
    try {
      session = _ports.restore(kept.link, secret);
    } on ServerError {
      _set(ServerLinked(link: kept.link, feeders: kept.feeders, problem: ServerProblem.other));
      return;
    }
    _session = session;
    for (final ListedFeeder feeder in kept.feeders) {
      try {
        _router?.rememberServerFeeder(feeder: feeder.identity, id: feeder.id);
      } on ServerError {
        // A kept id that does not parse: the next list replaces it.
      }
    }
    await _connect(session);
  }

  /// Read the account, hand the server to the router, list the feeders and
  /// follow the stream.
  Future<void> _connect(ServerSession session) async {
    final ServerAccount account;
    try {
      account = await session.account();
    } on ServerError catch (error) {
      _failed(error);
      return;
    }
    if (_disposed || _session != session) return;
    _router?.connectServer(session: session, account: account);
    if (_state case final ServerLinked linked) _set(linked.copyWith(account: account, problem: () => null));
    await refresh();
    _follow();
  }

  void _follow() {
    final PathRouter? router = _router;
    if (router == null || _following != null) return;
    _following = router.follow(lastEventId: _resume).listen(_heard);
  }

  void _heard(ServerUpdate update) {
    switch (update) {
      case ServerUpdate_Opened():
        if (_state case final ServerLinked linked) _set(linked.copyWith(problem: () => null));
      case ServerUpdate_Event(:final String resume, :final ServerEvent event):
        _resume = resume.isEmpty ? null : resume;
        switch (event) {
          case ServerEvent_Resync():
            unawaited(refresh());
          case ServerEvent_History() || ServerEvent_Connection():
            _events.add(event);
          case ServerEvent_Command() || ServerEvent_Install():
            // The router settles commands; installs have no screen here yet.
            break;
        }
      case ServerUpdate_Closed(:final ServerError? error):
        unawaited(_following?.cancel());
        _following = null;
        if (error != null) _failed(error);
    }
  }

  /// What a failed call says of the server: signed out stays until a login;
  /// anything else is tried again.
  void _failed(ServerError error) {
    final ServerProblem problem = switch (error.kind) {
      ServerErrorKind.tokenRevoked || ServerErrorKind.accountDisabled => ServerProblem.signedOut,
      ServerErrorKind.unreachable || ServerErrorKind.interrupted || ServerErrorKind.closed => ServerProblem.unreachable,
      _ => ServerProblem.other,
    };
    if (_state case final ServerLinked linked) _set(linked.copyWith(problem: () => problem));
    if (problem != ServerProblem.signedOut) {
      _again?.cancel();
      _again = Timer(retryWait, _retryNow);
    }
  }

  void _retryNow() {
    _again?.cancel();
    _again = null;
    final ServerSession? session = _session;
    if (session == null || _disposed) return;
    if (_state case ServerLinked(account: null)) {
      unawaited(_connect(session));
    } else {
      unawaited(refresh());
      _follow();
    }
  }

  /// Stop following and let go of the session, keeping nothing of the server.
  Future<void> _forget() async {
    await _drop();
    await _ports.forgetSecret();
    await _book.setServer(null);
    _set(const ServerAbsent());
  }

  Future<void> _drop() async {
    _again?.cancel();
    _again = null;
    _session?.close();
    _session = null;
    await _following?.cancel();
    _following = null;
    _resume = null;
    _router?.disconnectServer();
  }

  void _set(ServerState next) {
    _state = next;
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _again?.cancel();
    _session?.close();
    unawaited(_following?.cancel());
    unawaited(_events.close());
    super.dispose();
  }
}
