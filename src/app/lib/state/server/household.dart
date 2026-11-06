import 'package:flutter/foundation.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';

/// The home's pets and foods, as the server keeps them.
@immutable
final class HouseholdState {
  /// Nothing read yet.
  const HouseholdState({this.pets = const [], this.foods = const [], this.read = false, this.busy = false, this.error});

  /// The pets, as the server lists them.
  final List<ServerPet> pets;

  /// The foods, as the server lists them.
  final List<ServerFood> foods;

  /// [pets] and [foods] are what the server said, not the empty start.
  final bool read;

  /// A change is out.
  final bool busy;

  /// What the last read or change ran into; `null` once one goes through.
  final ServerError? error;

  /// This state with the named parts changed.
  HouseholdState copyWith({
    List<ServerPet>? pets,
    List<ServerFood>? foods,
    bool? read,
    bool? busy,
    ServerError? Function()? error,
  }) => HouseholdState(
    pets: pets == null ? this.pets : List.unmodifiable(pets),
    foods: foods == null ? this.foods : List.unmodifiable(foods),
    read: read ?? this.read,
    busy: busy ?? this.busy,
    error: error == null ? this.error : error(),
  );
}

/// The home's pets and foods on the owner's server. None of this reaches a
/// feeder: a pet's feeders and a feeder's food are the server's records.
/// A change the server answered no way is not sent again; the next read
/// shows whether it took.
class HouseholdFlow extends ChangeNotifier {
  /// The home on `_session`.
  HouseholdFlow(this._session);

  final ServerSession _session;
  bool _disposed = false;

  /// What the screens show.
  HouseholdState get state => _state;
  HouseholdState _state = const HouseholdState();

  /// Read the pets and foods.
  Future<void> load() async {
    try {
      final List<ServerPet> pets = await _session.pets();
      final List<ServerFood> foods = await _session.foods();
      _set(_state.copyWith(pets: pets, foods: foods, read: true, error: () => null));
    } on ServerError catch (error) {
      _set(_state.copyWith(error: () => error));
    }
  }

  /// Add a pet, or change `id`'s. A birth date is `YYYY-MM-DD`.
  Future<bool> savePet({
    String? id,
    required String name,
    required Species species,
    String? birthDate,
    int? dailyKcal,
  }) => _change(
    () => id == null
        ? _session.createPet(name: name, species: species, birthDate: birthDate, dailyKcal: dailyKcal)
        : _session.updatePet(id: id, name: name, species: species, birthDate: birthDate, dailyKcal: dailyKcal),
  );

  /// Remove pet `id`, with its weights.
  Future<bool> deletePet(String id) => _change(() => _session.deletePet(id: id));

  /// Add a food, or change `id`'s.
  Future<bool> saveFood({
    String? id,
    required String brand,
    required String name,
    required int kcalPerKg,
    required int bagGrams,
  }) => _change(
    () => id == null
        ? _session.createFood(brand: brand, name: name, kcalPerKg: kcalPerKg, bagGrams: bagGrams)
        : _session.updateFood(id: id, brand: brand, name: name, kcalPerKg: kcalPerKg, bagGrams: bagGrams),
  );

  /// Remove food `id`.
  Future<bool> deleteFood(String id) => _change(() => _session.deleteFood(id: id));

  /// Run one change, then read the lists again: whether the server took it.
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

  void _set(HouseholdState next) {
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
