import 'dart:async';

import 'package:flutter/foundation.dart';

/// A beat for what counts down on screen, once a second. The feeder's windows
/// are stated in seconds, so nothing shown needs finer.
class SecondTicker extends ChangeNotifier {
  /// Ticks until disposed.
  SecondTicker() {
    _timer = Timer.periodic(_second, (_) => notifyListeners());
  }

  static const _second = Duration(seconds: 1);

  late final Timer _timer;

  @override
  void dispose() {
    _timer.cancel();
    super.dispose();
  }
}

/// Whether a wait has gone on long enough to explain, for screens that stay
/// still through a slow step.
class Patience extends ChangeNotifier {
  /// Waiting `_after` from now.
  Patience(this._after) {
    restart();
  }

  final Duration _after;
  Timer? _timer;

  /// The wait has run its length.
  bool get long => _long;
  bool _long = false;

  /// Something moved: the wait starts over.
  void restart() {
    _long = false;
    _timer?.cancel();
    _timer = Timer(_after, () {
      _long = true;
      notifyListeners();
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }
}

/// When a wait ends. A timer, not a count of pauses: one read on a dead session
/// can take as long as the session takes to notice.
class Deadline {
  /// Ending `wait` from now.
  Deadline(Duration wait) {
    _timer = Timer(wait, () {
      _over = true;
      _done.complete();
    });
  }

  late final Timer _timer;
  final _done = Completer<void>();
  bool _over = false;

  /// The time ran out.
  bool get over => _over;

  /// Completes when the time runs out.
  Future<void> get done => _done.future;

  /// Stop waiting.
  void cancel() => _timer.cancel();

  /// A pause between looks: long enough not to crowd a session coming back, short
  /// enough that the owner isn't left watching a screen that has already moved on.
  Future<void> beat() => Future<void>.delayed(_beat);

  static const _beat = Duration(milliseconds: 500);
}
