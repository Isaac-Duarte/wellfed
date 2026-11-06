import 'package:flutter/foundation.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';

/// One feeder's food on the owner's server: what it holds, what is left, and
/// its refills.
@immutable
final class FeederFoodState {
  /// Nothing read yet.
  const FeederFoodState({this.food, this.foods = const [], this.refills = const [], this.busy = false, this.error});

  /// The server's view of the feeder's food; `null` before it is read.
  final ServerFeederFood? food;

  /// The home's foods, to pick the one the feeder holds.
  final List<ServerFood> foods;

  /// Its refills, newest first.
  final List<ServerRefill> refills;

  /// A change is out.
  final bool busy;

  /// What the last read or change ran into; `null` once one goes through.
  final ServerError? error;

  /// The feeder's portion calibration is set, so grams mean something. Until
  /// it is, every view counts portions: no grams, kcal or food left.
  bool get calibrated => food?.weight is PortionWeight_Calibration;

  /// This state with the named parts changed.
  FeederFoodState copyWith({
    ServerFeederFood? food,
    List<ServerFood>? foods,
    List<ServerRefill>? refills,
    bool? busy,
    ServerError? Function()? error,
  }) => FeederFoodState(
    food: food ?? this.food,
    foods: foods == null ? this.foods : List.unmodifiable(foods),
    refills: refills == null ? this.refills : List.unmodifiable(refills),
    busy: busy ?? this.busy,
    error: error == null ? this.error : error(),
  );
}

/// The food view of server feeder `_feeder`. These are the server's records:
/// none of it reaches the feeder. A refill whose answer was lost is read back
/// in Rust before it is made again, so a record is never doubled.
class FeederFoodFlow extends ChangeNotifier {
  /// The food of `_feeder` on `_session`.
  FeederFoodFlow(this._session, this._feeder);

  /// Refills read at a time.
  static const refillPage = 200;

  final ServerSession _session;
  final String _feeder;
  bool _disposed = false;

  /// What the screen shows.
  FeederFoodState get state => _state;
  FeederFoodState _state = const FeederFoodState();

  /// Read the food view, the foods and the newest refills.
  Future<void> load() async {
    try {
      final ServerFeederFood food = await _session.feederFood(feeder: _feeder);
      final List<ServerFood> foods = await _session.foods();
      final List<ServerRefill> refills = await _refills();
      _set(_state.copyWith(food: food, foods: foods, refills: refills, error: () => null));
    } on ServerError catch (error) {
      _set(_state.copyWith(error: () => error));
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
    } while (after != null && !_disposed);
    return oldestFirst.reversed.toList();
  }

  /// The feeder holds food `id` from now on.
  Future<bool> setFood(String id) => _change(() => _session.setFeederFood(feeder: _feeder, food: id));

  /// The owner filled it.
  Future<bool> addRefill(NewRefill refill) => _change(() => _session.addRefill(feeder: _feeder, refill: refill));

  /// Remove refill `id`, recorded by mistake.
  Future<bool> deleteRefill(String id) => _change(() => _session.deleteRefill(feeder: _feeder, refill: id));

  Future<bool> _change(Future<Object?> Function() change) async {
    if (_state.busy) return false;
    _set(_state.copyWith(busy: true));
    try {
      await change();
    } on ServerError catch (error) {
      _set(_state.copyWith(busy: false, error: () => error));
      return false;
    }
    _set(_state.copyWith(busy: false, error: () => null));
    await load();
    return true;
  }

  void _set(FeederFoodState next) {
    if (_disposed) return;
    _state = next;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
