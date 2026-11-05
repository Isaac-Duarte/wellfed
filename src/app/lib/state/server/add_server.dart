import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/state/server/server.dart';

/// Where adding a server stands.
sealed class AddServerStage {
  const AddServerStage();
}

/// Waiting for the owner's address, name and password.
final class AddServerAsking extends AddServerStage {
  /// The form, with nothing sent.
  const AddServerAsking();
}

/// The login is out.
final class AddServerLoggingIn extends AddServerStage {
  /// A login on its way.
  const AddServerLoggingIn();
}

/// The server took the login: the phone holds its own token now.
final class AddServerDone extends AddServerStage {
  /// A login that went through.
  const AddServerDone();
}

/// The server refused, or could not be reached; `error` says which.
final class AddServerFailed extends AddServerStage {
  /// A login that did not go through.
  const AddServerFailed(this.error);

  /// What the server, or the way to it, said.
  final ServerError error;
}

/// A server heard announcing itself on the LAN.
@immutable
final class HeardServer {
  /// The server at `url`, announced as `name`.
  const HeardServer({required this.url, required this.name});

  /// Where to log in.
  final String url;

  /// The name it announces, its label.
  final String name;
}

/// The add-server screen's state: the servers heard on the LAN, and the login.
@immutable
final class AddServerState {
  /// A form with `found` heard so far.
  const AddServerState({this.found = const [], this.stage = const AddServerAsking()});

  /// Servers announcing themselves, by URL, in the order heard.
  final List<HeardServer> found;

  /// How the login stands.
  final AddServerStage stage;
}

/// Adding the owner's server: listen on the LAN for one, then log in. The
/// password goes to the login and is kept nowhere.
class AddServerFlow extends ChangeNotifier {
  /// Listen for servers at once, until disposed.
  AddServerFlow(this._server) {
    _browsing = _server.browse().listen(
      _heard,
      onError: (Object _) {
        // No NSD here: the owner types the address instead.
      },
    );
  }

  final HomeServer _server;
  StreamSubscription<FoundServer>? _browsing;
  bool _disposed = false;

  /// What the screen shows.
  AddServerState get state => _state;
  AddServerState _state = const AddServerState();

  void _heard(FoundServer server) {
    if (_state.found.any((found) => found.url == server.url)) return;
    final heard = HeardServer(url: server.url, name: server.name);
    _set(AddServerState(found: List.unmodifiable([..._state.found, heard]), stage: _state.stage));
  }

  /// Log in to `url` as `name`. A URL with no scheme is taken as `http://`,
  /// as a LAN address usually is.
  Future<void> logIn({required String url, required String name, required String password}) async {
    if (_state.stage is AddServerLoggingIn) return;
    _stage(const AddServerLoggingIn());
    final String trimmed = url.trim();
    final String full = trimmed.contains('://') ? trimmed : 'http://$trimmed';
    final LoginOutcome outcome = await _server.logIn(url: full, name: name.trim(), password: password);
    if (_disposed) return;
    _stage(switch (outcome) {
      LoginDone() => const AddServerDone(),
      LoginFailed(:final ServerError error) => AddServerFailed(error),
    });
  }

  void _stage(AddServerStage stage) => _set(AddServerState(found: _state.found, stage: stage));

  void _set(AddServerState next) {
    _state = next;
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(_browsing?.cancel());
    super.dispose();
  }
}
