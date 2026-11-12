import 'package:wellfed/src/rust/api/server_error.dart';

/// How a logout ended.
sealed class LogOutOutcome {
  const LogOutOutcome();
}

/// The server is forgotten, and the phone keeps its keys on its feeders.
final class LoggedOut extends LogOutOutcome {
  /// A logout the server took, or could not be told of.
  const LoggedOut({required this.revoked});

  /// Whether the server took it: if not, the token works until it is
  /// revoked there.
  final bool revoked;
}

/// The password was refused, or too many were: nothing changed.
final class LogOutRefused extends LogOutOutcome {
  /// A logout the server refused.
  const LogOutRefused(this.error);

  /// What the server said.
  final ServerError error;
}
