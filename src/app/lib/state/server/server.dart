import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/platform/server_ports.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/server/log_in.dart';
import 'package:wellfed/state/server/log_out.dart';

export 'package:wellfed/state/server/log_in.dart';

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

  /// The kept address is plain `http://` beyond the home network, which
  /// this app no longer uses. The token already crossed the network in the
  /// clear and never expires, so the owner adds the server again by its
  /// `https://` address and revokes the old token on the server: removing
  /// the server here only forgets its secret.
  plainHttp,

  /// This phone's secure storage failed to read the server's token, which
  /// is still kept: a new start reads it again. Not signed out, so nothing
  /// asks for a login that would write over it.
  storage,
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
  const ServerLinked({
    required this.link,
    required this.feeders,
    this.others = const [],
    this.account,
    this.listed = false,
    this.problem,
  });

  /// Where it is, and as whom.
  final ServerLink link;

  /// Who the phone's token speaks for, once the server has said.
  final ServerAccount? account;

  /// The feeders it lists: the last list it gave, kept on the phone, until it
  /// gives a new one.
  final List<ListedFeeder> feeders;

  /// The devices it lists that this app does not serve as feeders, from
  /// its last list this run: each is shown with what it needs.
  final List<OtherDevice> others;

  /// [feeders] is what the server said since the app started, not only the
  /// kept copy.
  final bool listed;

  /// What stops it serving the phone now; `null` while it does.
  final ServerProblem? problem;

  /// The same server with the named parts changed.
  ServerLinked copyWith({
    ServerAccount? account,
    List<ListedFeeder>? feeders,
    List<OtherDevice>? others,
    bool? listed,
    ServerProblem? Function()? problem,
  }) => ServerLinked(
    link: link,
    account: account ?? this.account,
    feeders: feeders == null ? this.feeders : List.unmodifiable(feeders),
    others: others == null ? this.others : List.unmodifiable(others),
    listed: listed ?? this.listed,
    problem: problem == null ? this.problem : problem(),
  );
}

/// A device the server lists that this app does not serve as a feeder.
final class OtherDevice {
  /// The device named `name`, and what it is to this app.
  const OtherDevice({required this.name, required this.device});

  /// What the household calls it.
  final String name;

  /// Why the app does not serve it: never [ServerDevice.feeder].
  final ServerDevice device;
}

/// The owner's server: the login, the phone's token, the feeders it lists
/// and its live stream, which goes through the path router so the router
/// settles pinned writes by what the server says. The app with no server is
/// the app as it was: nothing here runs until a login.
class HomeServer extends ChangeNotifier {
  /// The server `_book` keeps, if any, served at once.
  ///
  /// The router is told of the kept server before anything is awaited, so
  /// no feed goes on the phone's own key that the kept account (or, with
  /// none kept, an account not yet read) would not allow. `_key` is this
  /// phone's own.
  HomeServer(this._book, this._ports, this._router, this._key) {
    final KnownServer? kept = _book.server;
    _routed = _router?.updates().listen(_routerSaid);
    if (kept != null) {
      _router?.configureServer(kept: kept);
      _state = ServerLinked(link: kept.link, feeders: kept.feeders, account: kept.account);
      unawaited(_restore(kept));
    }
  }

  /// A server that went quiet is tried again after this: the path router
  /// passes over a down server for as long before it tries it again.
  static const retryWait = Duration(seconds: 30);

  final FeederBook _book;
  final ServerPorts _ports;
  final PathRouter? _router;
  final ClientKey _key;

  /// The server as it stands.
  ServerState get state => _state;
  ServerState _state = const ServerAbsent();

  ServerSession? _session;
  StreamSubscription<ServerUpdate>? _following;
  StreamSubscription<RouterUpdate>? _routed;
  Future<void> _kept = Future.value();
  String? _resume;
  Timer? _again;
  bool _disposed = false;

  final _events = StreamController<ServerEvent>.broadcast();

  /// The feeders' events the server's stream brings, for the fleet: history
  /// entries and the server's links to them.
  Stream<ServerEvent> get events => _events.stream;

  /// The session the server's own screens read and write with: pets, foods,
  /// refills, weights and the long history, none of which reach a feeder.
  /// `null` with no server, or before a kept token is read.
  ServerSession? get session => _session;

  /// Owner's servers announcing themselves on the LAN, until cancelled.
  Stream<FoundServer> browse() => _ports.browse();

  /// The account may only look: no feeder takes a feed or a change from
  /// this phone, through the server or its own key. A role a newer server
  /// added grants nothing here, so it only looks too.
  bool get viewer => switch (_state) {
    ServerLinked(account: ServerAccount(role: ServerRole.viewer || ServerRole.unknown)) => true,
    _ => false,
  };

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

    _drop();
    final ServerLink link = session.link();
    // The router holds no key of the phone's own for this account until the
    // server says who it is: a viewer never feeds direct in between.
    _router?.configureServer(
      kept: KnownServer(link: link, feeders: const []),
    );
    _session = session;
    _set(ServerLinked(link: link, feeders: const []));
    // Through the queue, after any copy of the last server still writing;
    // then the secret, so a file with the old link never meets the new one.
    await _keep();
    try {
      await _ports.keepSecret(session.secret());
    } on Object catch (error) {
      debugPrint('the server token could not be kept: $error');
      // A token kept nowhere is lost at the next start: it goes now, login and all.
      try {
        await session.logout(password: password, key: _key);
      } on ServerError {
        // It stays valid until revoked on the server.
      }
      await _forget();
      return const LoginNotKept();
    }
    await _connect(session);
    return const LoginDone();
  }

  /// Revoke this phone's token with the account's `password`, then forget
  /// the server. The server keeps `key`, this phone's own, on its feeders
  /// with the account, and the keys of phones it invited; it revokes any
  /// other under the token. A wrong password, or too many, changes nothing;
  /// a server that could not take it is forgotten all the same, its token
  /// valid until it is revoked there.
  Future<LogOutOutcome> logOut(String password, ClientKey key) async {
    final ServerSession? session = _session;
    var revoked = false;
    if (session != null) {
      try {
        await session.logout(password: password, key: key);
        revoked = true;
      } on ServerError catch (error) {
        if (error.kind case ServerErrorKind.badCredentials || ServerErrorKind.rateLimited) {
          return LogOutRefused(error);
        }
      }
    }
    await _forget();
    return LoggedOut(revoked: revoked);
  }

  /// Forget the server on this phone alone, its token left as it is.
  Future<void> removeServer() => _forget();

  /// Read the server's feeders again.
  Future<void> refresh() async {
    final PathRouter? router = _router;
    final ServerSession? session = _session;
    if (router == null || session == null) return;
    final List<ServerFeeder> feeders;
    try {
      feeders = await router.refresh();
    } on ServerError catch (error) {
      // A read of a session since replaced says nothing of this one.
      if (_disposed || _session != session) return;
      _failed(error);
      return;
    }
    if (_disposed || _session != session) return;
    // The phone's own key under this token on each feeder, so revoking the
    // token revokes it; one that failed is tried again on the next list.
    unawaited(router.claimOwn(key: _key));

    final List<ServerFeeder> current = [
      for (final ServerFeeder feeder in feeders)
        if (feeder.archivedMs == null) feeder,
    ];
    final List<ListedFeeder> listed = [
      for (final ServerFeeder feeder in current)
        if (feeder.device == ServerDevice.feeder)
          ListedFeeder(id: feeder.id, identity: feeder.identity, name: feeder.name),
    ];
    final List<OtherDevice> others = [
      for (final ServerFeeder feeder in current)
        switch (feeder.device) {
          ServerDevice.feeder => null,
          ServerDevice.otherKind || ServerDevice.newer => OtherDevice(name: feeder.name, device: feeder.device),
        },
    ].nonNulls.toList();
    if (_state case final ServerLinked linked) {
      _set(linked.copyWith(feeders: listed, others: others, listed: true, problem: () => null));
      await _keep();
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

  /// Keep the server as it stands now in the app's file, for a cold start:
  /// one write at a time, each of the state when it runs, so a slower write
  /// never leaves an older account behind.
  Future<void> _keep() => _kept = _kept.then((_) async {
    final ServerState state = _state;
    if (state is! ServerLinked || _disposed) return;
    try {
      await _book.setServer(KnownServer(link: state.link, feeders: state.feeders, account: state.account));
    } on Object catch (error) {
      // Only the copy for a cold start; the next read keeps it.
      debugPrint('the server was not kept: $error');
    }
  });

  /// The router read the account again (a list, or the stream reopening):
  /// a role or expiry the server changed is kept for the next cold start.
  void _routerSaid(RouterUpdate update) {
    if (update case RouterUpdate_Account(:final ServerAccount account)) {
      if (_state case final ServerLinked linked) {
        _set(linked.copyWith(account: account));
        unawaited(_keep());
      }
    }
  }

  String get _phone => _book.label.isEmpty ? 'Wellfed phone' : _book.label;

  Future<void> _restore(KnownServer kept) async {
    final String? secret;
    try {
      secret = await _ports.secret();
    } on Object catch (error) {
      // The phone's own keys stay closed, as with any server it cannot use.
      debugPrint('the server token could not be read: $error');
      if (_disposed) return;
      _router?.refuseServer();
      _set(ServerLinked(link: kept.link, feeders: kept.feeders, problem: ServerProblem.storage));
      return;
    }
    if (_disposed) return;
    // With no token the phone has no standing on its server, and with a
    // link that does not open it cannot ask: either way the router keeps
    // the phone's own keys closed until a login or the server's removal.
    if (secret == null) {
      _router?.refuseServer();
      _set(ServerLinked(link: kept.link, feeders: kept.feeders, problem: ServerProblem.signedOut));
      return;
    }
    final ServerSession session;
    try {
      session = _ports.restore(kept.link, secret);
    } on ServerError catch (error) {
      _router?.refuseServer();
      final ServerProblem problem = error.kind == ServerErrorKind.plainHttp
          ? ServerProblem.plainHttp
          : ServerProblem.other;
      _set(ServerLinked(link: kept.link, feeders: kept.feeders, problem: problem));
      return;
    }
    _session = session;
    await _connect(session);
  }

  /// Hand the server to the router, which reads the account and decides by
  /// it (or, with the server down, by the account last kept) which keys the
  /// phone may use; then list the feeders and follow the stream.
  Future<void> _connect(ServerSession session) async {
    final PathRouter? router = _router;
    final ServerAccount account;
    try {
      account = router == null
          ? await session.account()
          : await router.resumeServer(session: session, cached: _book.server?.account);
    } on ServerError catch (error) {
      if (_disposed || _session != session) return;
      _failed(error);
      return;
    }
    if (_disposed || _session != session) return;
    if (_state case final ServerLinked linked) {
      _set(linked.copyWith(account: account, problem: () => null));
      // Kept, for a cold start with the server down to go by.
      await _keep();
    }
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
    _drop();
    _set(const ServerAbsent());
    // A copy kept before this writes first, so none outlives the removal.
    await _kept;
    try {
      await _ports.forgetSecret();
    } on Object catch (error) {
      // The file forgets it all the same: a token beside no server is never read.
      debugPrint('the server token could not be forgotten: $error');
    }
    await _book.setServer(null);
  }

  void _drop() {
    _again?.cancel();
    _again = null;
    _session?.close();
    _session = null;
    // No event comes once it is cancelled; its end is not waited for, so a
    // stream slow to close never holds the logout up.
    unawaited(_following?.cancel());
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
    unawaited(_routed?.cancel());
    unawaited(_events.close());
    super.dispose();
  }
}
