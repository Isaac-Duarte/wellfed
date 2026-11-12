import 'package:wellfed/src/rust/api/server_views.dart';

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

  /// The kept address is plain `http://` off this device, which this app
  /// no longer uses. The token already crossed the network in the
  /// clear and never expires, so the owner adds the server again by its
  /// `https://` address and revokes the old token on the server: removing
  /// the server here only forgets its secret.
  plainHttp,

  /// This phone's secure storage failed to read the server's token, which
  /// is still kept: a new start reads it again. Not signed out, so nothing
  /// asks for a login that would write over it.
  storage,

  /// The server at the kept address proved another identity key than the
  /// one pinned when it was added: another machine answers there, or the
  /// server was set up again. Nothing was sent to it, the token included,
  /// and nothing is tried again: the only way on is to remove the server
  /// and add it again, once the owner knows why the key changed. The phone
  /// reaches its own feeders directly meanwhile.
  keyChanged,
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
