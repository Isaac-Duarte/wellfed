import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/platform/secure_storage.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_views.dart';

/// What the owner's server's controller reaches outside Dart: the Rust
/// session, and secure storage for the phone's token. Behind this, so the
/// account screens are tested with no Rust and no network.
abstract interface class ServerPorts {
  /// Log in to the server at `url` as `name`, and mint a token for this phone
  /// named `phone`. The password goes no further than this call.
  Future<ServerSession> login({
    required String url,
    required String name,
    required String password,
    required String phone,
  });

  /// The session a kept link and token secret make; nothing is sent yet.
  ServerSession restore(ServerLink link, String secret);

  /// The phone token's secret, as secure storage keeps it.
  Future<String?> secret();

  /// Keep the phone token's secret in secure storage.
  Future<void> keepSecret(String secret);

  /// Forget the phone token's secret.
  Future<void> forgetSecret();

  /// Owner's servers announcing themselves on the LAN, until cancelled.
  Stream<FoundServer> browse();
}

/// [ServerPorts] on every platform: the token's secret beside the client
/// secret, in [secureStorage].
class AppServerPorts implements ServerPorts {
  /// Ports for this phone.
  const AppServerPorts();

  static const _secretName = 'server-token';

  @override
  Future<ServerSession> login({
    required String url,
    required String name,
    required String password,
    required String phone,
  }) => ServerSession.login(url: url, name: name, password: password, phone: phone);

  @override
  ServerSession restore(ServerLink link, String secret) => ServerSession.restore(link: link, secret: secret);

  @override
  Future<String?> secret() => secureStorage.read(key: _secretName);

  @override
  Future<void> keepSecret(String secret) => secureStorage.write(key: _secretName, value: secret);

  @override
  Future<void> forgetSecret() => secureStorage.delete(key: _secretName);

  @override
  Stream<FoundServer> browse() => Discovery.browseServers();
}
