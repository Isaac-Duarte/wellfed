import 'dart:async';

import 'package:wellfed/src/rust/api/alerts.dart' show AlertHold;

/// The runs that hold this process up (an install, an update, a restore),
/// each under its own key, so one that ends never lets go of another's hold.
/// The keep-alive service and the screen stay on while any run holds, and
/// say the newest run's words; catch-ups stay off each feeder meanwhile.
class RunHolds {
  /// Holds that reach the platform through `_apply` (the newest run's words,
  /// or `null` once none holds: whether Android kept the process) and Rust's
  /// catch-up hold through `_mark`.
  RunHolds(this._apply, this._mark);

  final Future<bool> Function(String? text) _apply;
  final void Function(AlertHold run, {required bool held}) _mark;

  /// Each run holding now, oldest first, with its words.
  final _held = <AlertHold, String>{};

  /// The runs holding now.
  Set<AlertHold> get held => Set.unmodifiable(_held.keys);

  final _lapsed = StreamController<void>.broadcast();

  /// Each time the hold gave way while runs still held: a computer's helper
  /// that ended by itself. A run's screen then says to keep it awake.
  Stream<void> get lapsed => _lapsed.stream;

  /// The hold gave way under the runs holding now. Nothing while none holds.
  void lapse() {
    if (_held.isNotEmpty) _lapsed.add(null);
  }

  /// Hold the process for `run`, saying `text`. Whether Android went along.
  Future<bool> hold(AlertHold run, String text) {
    _held.remove(run);
    _held[run] = text;
    _mark(run, held: true);
    return _apply(text);
  }

  /// `run` is done. The process stays held while another run is.
  Future<void> release(AlertHold run) async {
    if (_held.remove(run) == null) return;
    _mark(run, held: false);
    await _apply(_held.values.lastOrNull);
  }
}
