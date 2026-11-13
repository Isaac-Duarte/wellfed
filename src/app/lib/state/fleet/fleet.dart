import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/feeder_ports.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/app_state.dart';

/// One controller per feeder, kept while the app runs, so every screen of a
/// feeder shows the one session: the feeders this phone holds its own key on.
class FeederFleet extends ChangeNotifier {
  /// One controller for each feeder in `_book`, kept in step with it.
  FeederFleet(this._book, this._ports) {
    _book.addListener(_sync);
    _routed = _ports.router?.updates().listen(_routerSaid);
    _sync();
  }

  final FeederBook _book;
  final FeederPorts _ports;
  final _feeders = <String, FeederController>{};
  StreamSubscription<RouterUpdate>? _routed;

  /// Every feeder's controller.
  Iterable<FeederController> get all => _feeders.values;

  /// The controller of `identity`, if this phone knows it.
  FeederController? operator [](String identity) => _feeders[identity];

  /// The app is back in the foreground: every feeder tries its link now.
  void resumed() {
    for (final FeederController controller in _feeders.values) {
      controller.resumed();
    }
  }

  void _sync() {
    final Map<String, KnownFeeder> own = {for (final f in _book.feeders) f.identity: f};

    for (final String identity in _feeders.keys.toList()) {
      if (!own.containsKey(identity)) _feeders.remove(identity)?.dispose();
    }
    for (final KnownFeeder feeder in own.values) {
      // One already serving follows the file: a re-pair renames it.
      _feeders[feeder.identity]?.knownChanged(feeder);
      _feeders.putIfAbsent(feeder.identity, () => FeederController(_book, _ports, feeder));
    }
    notifyListeners();
  }

  void _routerSaid(RouterUpdate update) => _feeders[update.pending.feeder]?.routerSaid(update);

  @override
  void dispose() {
    _book.removeListener(_sync);
    unawaited(_routed?.cancel());
    for (final FeederController controller in _feeders.values) {
      controller.dispose();
    }
    super.dispose();
  }
}
