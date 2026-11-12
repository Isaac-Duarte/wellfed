import 'package:flutter/foundation.dart';
import 'package:wellfed/src/rust/api/server_error.dart';

/// What a records screen's last read or change ran into.
@immutable
final class RecordProblem {
  /// `error`, from a read, or from a change when `change` is set.
  const RecordProblem(this.error, {this.change = false});

  /// What the server, or the way to it, said.
  final ServerError error;

  /// It ended a change, not a read.
  final bool change;

  /// A change that may have run all the same: its answer was lost, or the
  /// server failed partway. The records were read again after it, so what
  /// the screen lists says whether it took; nothing sends it again, and the
  /// owner checks before asking anew.
  bool get mayHaveRun =>
      change &&
      switch (error.kind) {
        ServerErrorKind.interrupted || ServerErrorKind.serverFailed || ServerErrorKind.outcomeUnknown => true,
        _ => false,
      };

  @override
  bool operator ==(Object other) => other is RecordProblem && other.error == error && other.change == change;

  @override
  int get hashCode => Object.hash(error, change);
}

/// A records state's part a [RecordsFlow] keeps: whether a change is out,
/// and what the last read or change ran into.
abstract interface class RecordsState<S> {
  /// A change is out.
  bool get busy;

  /// What the last read or change ran into; `null` once one goes through.
  RecordProblem? get problem;

  /// This state, with a change out or not, and `problem`.
  S ran({required bool busy, required RecordProblem? problem});
}

/// A screen of the owner's server's records: pets, foods, refills and
/// weights. None of it reaches a feeder. One change goes at a time and the
/// records are read again after it, whether or not it took. A change whose
/// answer never came is not sent again: the screen says it may have run,
/// and the read after it shows whether it did.
abstract class RecordsFlow<S extends RecordsState<S>> extends ChangeNotifier {
  /// Starting from `state`.
  RecordsFlow(S state) : _state = state;

  S _state;
  bool _disposed = false;

  /// What the screen shows.
  S get state => _state;

  /// The screen has gone: nothing more is shown.
  bool get disposed => _disposed;

  /// Read the records again.
  Future<void> load();

  /// Show `next`, unless the screen has gone.
  @protected
  void show(S next) {
    if (_disposed) return;
    _state = next;
    notifyListeners();
  }

  /// A read ran into `error`.
  @protected
  void readFailed(ServerError error) => show(_state.ran(busy: _state.busy, problem: RecordProblem(error)));

  /// Run `change` once, unless another is out, and hand what it answered to
  /// `took`; whether it took. The records are read again after it, and
  /// after one that may have run too.
  @protected
  Future<bool> change<T>(Future<T> Function() change, {void Function(T answer)? took}) async {
    if (_state.busy) return false;
    show(_state.ran(busy: true, problem: null));

    final T answer;
    try {
      answer = await change();
    } on ServerError catch (error) {
      final problem = RecordProblem(error, change: true);
      if (problem.mayHaveRun) await load();
      show(_state.ran(busy: false, problem: problem));
      return false;
    }

    took?.call(answer);
    show(_state.ran(busy: false, problem: null));
    await load();
    return true;
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
