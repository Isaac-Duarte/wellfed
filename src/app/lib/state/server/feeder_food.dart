import 'package:flutter/foundation.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/server/records.dart';

/// One feeder's food on the owner's server: what it holds, what is left, and
/// its refills.
@immutable
final class FeederFoodState implements RecordsState<FeederFoodState> {
  /// Nothing read yet.
  const FeederFoodState({this.food, this.foods = const [], this.refills = const [], this.busy = false, this.problem});

  /// The server's view of the feeder's food; `null` before it is read.
  final ServerFeederFood? food;

  /// The home's foods, to pick the one the feeder holds.
  final List<ServerFood> foods;

  /// Its refills, newest first.
  final List<ServerRefill> refills;

  @override
  final bool busy;

  @override
  final RecordProblem? problem;

  /// The feeder's portion calibration is set, so grams mean something. Until
  /// it is, every view counts portions: no grams, kcal or food left.
  bool get calibrated => food?.weight is PortionWeight_Calibration;

  /// This state with the named parts changed.
  FeederFoodState copyWith({ServerFeederFood? food, List<ServerFood>? foods, List<ServerRefill>? refills}) =>
      FeederFoodState(
        food: food ?? this.food,
        foods: foods == null ? this.foods : List.unmodifiable(foods),
        refills: refills == null ? this.refills : List.unmodifiable(refills),
        busy: busy,
        problem: problem,
      );

  @override
  FeederFoodState ran({required bool busy, required RecordProblem? problem}) =>
      FeederFoodState(food: food, foods: foods, refills: refills, busy: busy, problem: problem);
}

/// The food view of server feeder `_feeder`. These are the server's records:
/// none of it reaches the feeder.
class FeederFoodFlow extends RecordsFlow<FeederFoodState> {
  /// The food of `_feeder` on `_session`.
  FeederFoodFlow(this._session, this._feeder) : super(const FeederFoodState());

  /// Refills read at a time.
  static const refillPage = 200;

  final ServerSession _session;
  final String _feeder;

  @override
  Future<void> load() async {
    try {
      final ServerFeederFood food = await _session.feederFood(feeder: _feeder);
      final List<ServerFood> foods = await _session.foods();
      final List<ServerRefill> refills = await _refills();
      show(state.copyWith(food: food, foods: foods, refills: refills).ran(busy: state.busy, problem: null));
    } on ServerError catch (error) {
      readFailed(error);
    }
  }

  /// The server pages oldest first and the screen lists newest first, so
  /// every page is read (a feeder is filled a few times a month).
  Future<List<ServerRefill>> _refills() async {
    final oldestFirst = <ServerRefill>[];
    String? after;
    do {
      final ServerRefillPage page = await _session.refills(feeder: _feeder, after: after, limit: refillPage);
      oldestFirst.addAll(page.items);
      if (page.next == after || page.items.isEmpty) break;
      after = page.next;
    } while (after != null && !disposed);
    return oldestFirst.reversed.toList();
  }

  /// The feeder holds food `id` from now on.
  Future<bool> setFood(String id) => change(() => _session.setFeederFood(feeder: _feeder, food: id));

  /// The owner filled it.
  Future<bool> addRefill(NewRefill refill) => change(() => _session.addRefill(feeder: _feeder, refill: refill));

  /// Remove refill `id`, recorded by mistake.
  Future<bool> deleteRefill(String id) => change(() => _session.deleteRefill(feeder: _feeder, refill: id));
}
