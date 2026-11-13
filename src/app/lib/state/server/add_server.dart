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

/// The server took the login, and this phone could not keep its token.
final class AddServerNotKept extends AddServerStage {
  /// A login kept nowhere.
  const AddServerNotKept();
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
  /// The server at `url`, announced as `name` with identity `key`.
  const HeardServer({required this.url, required this.name, required this.key, this.fingerprint});

  /// Where to log in.
  final String url;

  /// The name it announces, its label.
  final String name;

  /// The identity key it announces, which the login checks.
  final String key;

  /// What a person compares of [key] with what the server logs; `null`
  /// for a key that does not read.
  final String? fingerprint;
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
    final heard = HeardServer(
      url: server.url,
      name: server.name,
      key: server.key,
      fingerprint: _server.fingerprint(server.key),
    );
    _set(AddServerState(found: List.unmodifiable([..._state.found, heard]), stage: _state.stage));
  }

  /// Log in to `url` as `name`. A URL with no scheme gets one in Rust:
  /// `https://`, plain `http://` only on this device itself. A server heard
  /// at `url` must prove the key it announced; a typed one is pinned by its
  /// first handshake.
  Future<void> logIn({required String url, required String name, required String password}) async {
    if (_state.stage is AddServerLoggingIn) return;
    _stage(const AddServerLoggingIn());
    final String typed = url.trim();
    final String? key = _state.found.where((found) => found.url == typed).firstOrNull?.key;
    final LoginOutcome outcome = await _server.logIn(url: typed, name: name.trim(), password: password, key: key);
    if (_disposed) return;
    if (outcome is LoginDone) {
      // Nothing listens for another server once one is in: a server heard
      // now would only stir a screen on its way out.
      unawaited(_browsing?.cancel());
      _browsing = null;
    }
    _stage(switch (outcome) {
      LoginDone() => const AddServerDone(),
      LoginNotKept() => const AddServerNotKept(),
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
