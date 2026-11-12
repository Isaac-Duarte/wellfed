import 'package:wellfed/src/rust/api/server_error.dart';

/// How a login ended.
sealed class LoginOutcome {
  const LoginOutcome();
}

/// Logged in: the phone's token is kept, and the password is not.
final class LoginDone extends LoginOutcome {
  /// A login that went through.
  const LoginDone();
}

/// The server took the login, but this phone's storage could not keep its
/// token: nothing of the login is kept, and the token is revoked if the
/// server takes that.
final class LoginNotKept extends LoginOutcome {
  /// A login with nowhere to keep its token.
  const LoginNotKept();
}

/// The server refused or could not be reached; `error` says which.
final class LoginFailed extends LoginOutcome {
  /// A login that did not go through.
  const LoginFailed(this.error);

  /// What the server, or the way to it, said.
  final ServerError error;
}
