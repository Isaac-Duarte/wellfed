import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/feeder_ports.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/server/server.dart';

/// One controller per feeder, kept while the app runs, so every screen of a
/// feeder shows the one session. With no server these are the feeders this
/// phone holds its own key on. With one, they are the server's list, the
/// phone's own key used where it holds one, and any feeder it paired that
/// the server does not list, which is offered for adoption.
class FeederFleet extends ChangeNotifier {
  /// One controller for each feeder in `_book` and on `_server`, kept in step
  /// with both.
  FeederFleet(this._book, this._ports, {this._server}) {
    _book.addListener(_sync);
    _server?.addListener(_sync);
    _routed = _ports.router?.updates().listen(_routerSaid);
    _heard = _server?.events.listen(_serverSaid);
    _sync();
  }

  final FeederBook _book;
  final FeederPorts _ports;
  final HomeServer? _server;
  final _feeders = <String, FeederController>{};
  StreamSubscription<RouterUpdate>? _routed;
  StreamSubscription<ServerEvent>? _heard;

  /// The server ids the details were last read for.
  Set<String> _detailed = const {};

  /// Every feeder's controller.
  Iterable<FeederController> get all => _feeders.values;

  /// The controller of `identity`, if this phone knows it.
  FeederController? operator [](String identity) => _feeders[identity];

  /// The feeders this phone paired that the server it is logged in to does
  /// not list, once the server has given its list: each can be adopted.
  List<KnownFeeder> get adoptable {
    final ServerState? server = _server?.state;
    if (server is! ServerLinked || !server.listed) return const [];
    final Set<String> listed = {for (final ListedFeeder feeder in server.feeders) feeder.identity};
    return List.unmodifiable(_book.feeders.where((feeder) => !listed.contains(feeder.identity)));
  }

  /// The app is back in the foreground: every feeder tries its link now.
  void resumed() {
    _server?.resumed();
    for (final FeederController controller in _feeders.values) {
      controller.resumed();
    }
  }

  List<ListedFeeder> get _listed => switch (_server?.state) {
    ServerLinked(:final List<ListedFeeder> feeders) => feeders,
    _ => const [],
  };

  void _sync() {
    final Map<String, KnownFeeder> own = {for (final f in _book.feeders) f.identity: f};
    final Map<String, ListedFeeder> listed = {for (final f in _listed) f.identity: f};

    for (final String identity in _feeders.keys.toList()) {
      final FeederController? controller = _feeders[identity];
      final bool ownKey = own.containsKey(identity);
      // Gone, or the phone's key came or went: a controller of the other kind.
      if ((!ownKey && !listed.containsKey(identity)) || controller?.ownKey != ownKey) {
        _feeders.remove(identity)?.dispose();
      }
    }
    for (final KnownFeeder feeder in own.values) {
      _feeders.putIfAbsent(feeder.identity, () => FeederController(_book, _ports, feeder));
    }
    for (final ListedFeeder feeder in listed.values) {
      _feeders.putIfAbsent(
        feeder.identity,
        () => FeederController(_book, _ports, KnownFeeder(identity: feeder.identity, name: feeder.name), ownKey: false),
      );
    }

    final ClientRole? role = _server?.feederRole;
    for (final MapEntry(key: identity, value: controller) in _feeders.entries) {
      controller.serverListed(listed[identity]?.id, role: role);
      if (_ports.router?.path(feeder: identity) case final RoutePath path) controller.routed(path);
    }
    _readDetails(listed.values.map((feeder) => feeder.id).toSet());
    notifyListeners();
  }

  /// Read the server's view of each feeder it lists anew, once per list.
  void _readDetails(Set<String> ids) {
    if (setEquals(ids, _detailed)) return;
    _detailed = ids;
    for (final String id in ids) {
      unawaited(_readDetail(id));
    }
  }

  Future<void> _readDetail(String id) async {
    final ServerFeederDetail? detail = await _server?.detail(id);
    if (detail == null) return;
    _byServerId(id)?.serverDetail(detail);
  }

  FeederController? _byServerId(String id) {
    for (final FeederController controller in _feeders.values) {
      if (controller.state.serverId == id) return controller;
    }
    return null;
  }

  void _routerSaid(RouterUpdate update) {
    final String? feeder = switch (update) {
      RouterUpdate_Path(:final String feeder) => feeder,
      RouterUpdate_Settled(:final PendingWrite pending) => pending.feeder,
      RouterUpdate_FeedEnded(:final String feeder) => feeder,
      // The account is the server's to keep.
      RouterUpdate_Account() => null,
    };
    if (feeder != null) _feeders[feeder]?.routerSaid(update);
  }

  void _serverSaid(ServerEvent event) {
    switch (event) {
      case ServerEvent_History(:final String feeder, :final ServerEntry entry):
        _byServerId(feeder)?.serverHistory(entry.entry);
        // A new event may come with a new status: read it again.
        unawaited(_readDetail(feeder));
      case ServerEvent_Connection(:final String feeder, :final ServerConnection connection):
        _byServerId(feeder)?.serverConnection(connection);
        unawaited(_readDetail(feeder));
      case _:
        break;
    }
  }

  @override
  void dispose() {
    _book.removeListener(_sync);
    _server?.removeListener(_sync);
    unawaited(_routed?.cancel());
    unawaited(_heard?.cancel());
    for (final FeederController controller in _feeders.values) {
      controller.dispose();
    }
    super.dispose();
  }
}
