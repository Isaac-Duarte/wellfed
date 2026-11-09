part of 'feeder.dart';

/// The feeder as the owner's server last told it.
final class _ServerSide {
  const _ServerSide({required this.id, this.link, this.card, this.at, this.live = false, this.role, this.skew});

  /// The server's id for it.
  final String id;

  /// The server's own link to it; `null` before the server said.
  final ServerConnection? link;

  /// Its status as the server last heard it, with no plans, settings or time.
  final Card? card;

  /// When the server heard [card].
  final DateTime? at;

  /// The server's session with it is open now, and [card] is that session's.
  final bool live;

  /// What this account may do there, as a feeder's own role names it; `null`
  /// for a viewer.
  final ClientRole? role;

  /// How the feeder's API revision stands against the server's.
  final Skew? skew;

  _ServerSide copyWith({ServerConnection? link, Card? card, DateTime? at, bool? live, ClientRole? role, Skew? skew}) =>
      _ServerSide(
        id: id,
        link: link ?? this.link,
        card: card ?? this.card,
        at: at ?? this.at,
        live: live ?? this.live,
        role: role ?? this.role,
        skew: skew ?? this.skew,
      );
}

/// What the fleet hears of this feeder from the owner's server and the path
/// router, and how [FeederController.state] shows it: the phone's own session
/// first, and the server's view while that session is down and the server's
/// is up, so a feeder away from the LAN still reads and feeds.
extension FeederServerSide on FeederController {
  /// The server lists it as `id`, for an account whose standing there a
  /// feeder's role names as `role` (`null` for a viewer); `id` `null` when it
  /// no longer does, or there is no server. A `viewer` account feeds no
  /// feeder, listed or not: its phone's own keys are closed too.
  void serverListed(String? id, {ClientRole? role, bool viewer = false}) {
    _viewer = viewer;
    final _ServerSide? before = _server;
    _server = id == null
        ? null
        : before != null && before.id == id
        ? _ServerSide(
            id: id,
            link: before.link,
            card: before.card,
            at: before.at,
            live: before.live,
            role: role,
            skew: before.skew,
          )
        : _ServerSide(id: id, role: role);
    _setState(_state);
  }

  /// The feeder's status and link, as the server read them.
  void serverDetail(ServerFeederDetail detail) {
    final _ServerSide? server = _server;
    if (server == null || server.id != detail.feeder.id) return;
    final ServerSnapshot? status = detail.status;
    final bool online = detail.feeder.connection == ServerConnection.online;
    _server = server.copyWith(
      link: detail.feeder.connection,
      skew: detail.feeder.skew,
      card: status?.card,
      at: status == null ? null : DateTime.fromMillisecondsSinceEpoch(status.atMs),
      live: online && (status?.live ?? false),
    );
    _setState(_state);
  }

  /// The server's link to the feeder changed. Its card is only live again
  /// once the server reads the status anew ([serverDetail]).
  void serverConnection(ServerConnection link) {
    final _ServerSide? server = _server;
    if (server == null) return;
    _server = server.copyWith(link: link, live: link == ServerConnection.online && server.live);
    _setState(_state);
  }

  /// One entry of the feeder's history the server just received. A feed of
  /// this phone's that went through the server ends here. With no session of
  /// the phone's own, it is the history this phone has.
  void serverHistory(HistoryEntry entry) {
    if (entry.body case HistoryBody_Fed(:final FeedResult result)) _feedEnded(result);
    if (!_ownKey) _setState(_state.copyWith(history: [..._state.history, entry]));
  }

  /// The way a new request takes now, as the router says.
  void routed(RoutePath? path) => _setState(_state.copyWith(path: () => path));

  /// [next], showing the server's view while the phone's own session is down
  /// and the server's link is up.
  FeederState _reached(FeederState next) {
    final _ServerSide? server = _server;
    final FeederState listed = next.copyWith(
      serverId: () => server?.id,
      viewer: _viewer,
      serverSkew: () => server?.skew,
    );
    final Card? card = server?.card;
    if (_connection == Connection.online || server == null || !server.live || card == null) {
      return listed.copyWith(connection: _connection);
    }
    return listed.copyWith(
      connection: Connection.online,
      card: card,
      cardAt: server.at,
      cardFresh: true,
      role: next.role ?? server.role,
    );
  }
}
