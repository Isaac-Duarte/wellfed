import 'dart:async';

import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';

/// How long [invitedPhones] waits: the owner waits on it to log out.
const invitedWait = Duration(seconds: 5);

/// The phones this phone invited on its feeders, which a logout revokes with
/// the token: the server's id of each one's feeder, as `session`'s server
/// recorded them under this phone's token beside `key`, its own. `null` with
/// no session, or when the server did not say within [invitedWait].
Future<List<String>?> invitedPhones(ServerSession? session, ClientKey key) async {
  if (session == null) return null;
  try {
    return await session.invitedPhones(key: key).timeout(invitedWait);
  } on ServerError {
    return null;
  } on TimeoutException {
    return null;
  }
}
