import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/platform/host.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/store.dart';

/// How often a computer catches up while the app is open: the minute of
/// Android's "instant while home", as a computer at home is on its network
/// and has no battery to spare it for.
const computerRoundEvery = Duration(minutes: 1);

/// A computer's catch-ups while the app is open, the way Android's instant
/// mode runs them: a round now and each [computerRoundEvery], one at a time,
/// so a slow round (a feeder gone, discovery looking) makes the next minute's
/// skip rather than pile up. A failed round is logged; the next one is the
/// retry, and it reads only, so it can't repeat anything.
class AlertWatch {
  /// Watching through `_round`, every `_every`.
  AlertWatch(this._round, {this._every = computerRoundEvery});

  /// [AlertWatch] over this computer's feeders, as `key`: each round reads the
  /// feeders the app knows at that moment.
  factory AlertWatch.computer(ClientKey key, List<KnownFeeder> Function() feeders) => AlertWatch(() async {
    await catchUpRound(
      key: key,
      dir: (await appDataDirectory()).path,
      feeders: feeders(),
      network: computerNetwork,
      failures: RoundFailures.logged,
    );
  });

  final Future<void> Function() _round;
  final Duration _every;
  Timer? _timer;
  bool _running = false;

  /// Rounds run.
  void start() {
    if (_timer != null) return;
    unawaited(_tick());
    _timer = Timer.periodic(_every, (_) => unawaited(_tick()));
  }

  /// No more rounds; one running ends by itself.
  void stop() {
    _timer?.cancel();
    _timer = null;
  }

  Future<void> _tick() async {
    if (_running) return;
    _running = true;
    try {
      await _round();
    } on Object catch (error) {
      debugPrint('a round of alerts failed: $error');
    } finally {
      _running = false;
    }
  }
}
