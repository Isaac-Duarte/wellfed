import 'package:flutter/foundation.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/server/records.dart';

/// The home's pets and foods, as the server keeps them.
@immutable
final class HouseholdState implements RecordsState<HouseholdState> {
  /// Nothing read yet.
  const HouseholdState({
    this.pets = const [],
    this.foods = const [],
    this.read = false,
    this.busy = false,
    this.problem,
  });

  /// The pets, as the server lists them.
  final List<ServerPet> pets;

  /// The foods, as the server lists them.
  final List<ServerFood> foods;

  /// [pets] and [foods] are what the server said, not the empty start.
  final bool read;

  @override
  final bool busy;

  @override
  final RecordProblem? problem;

  /// This state with the named parts changed.
  HouseholdState copyWith({List<ServerPet>? pets, List<ServerFood>? foods, bool? read}) => HouseholdState(
    pets: pets == null ? this.pets : List.unmodifiable(pets),
    foods: foods == null ? this.foods : List.unmodifiable(foods),
    read: read ?? this.read,
    busy: busy,
    problem: problem,
  );

  @override
  HouseholdState ran({required bool busy, required RecordProblem? problem}) =>
      HouseholdState(pets: pets, foods: foods, read: read, busy: busy, problem: problem);
}

/// The home's pets and foods on the owner's server. None of this reaches a
/// feeder: a pet's feeders and a feeder's food are the server's records.
class HouseholdFlow extends RecordsFlow<HouseholdState> {
  /// The home on `_session`.
  HouseholdFlow(this._session) : super(const HouseholdState());

  final ServerSession _session;

  @override
  Future<void> load() async {
    try {
      final List<ServerPet> pets = await _session.pets();
      final List<ServerFood> foods = await _session.foods();
      show(state.copyWith(pets: pets, foods: foods, read: true).ran(busy: state.busy, problem: null));
    } on ServerError catch (error) {
      readFailed(error);
    }
  }

  /// Add a pet. A birth date is `YYYY-MM-DD`.
  Future<bool> addPet({required String name, required Species species, String? birthDate, int? dailyKcal}) =>
      change(() => _session.createPet(name: name, species: species, birthDate: birthDate, dailyKcal: dailyKcal));

  /// Add a food, or change `id`'s.
  Future<bool> saveFood({
    String? id,
    required String brand,
    required String name,
    required int kcalPerKg,
    required int bagGrams,
  }) => change(
    () => id == null
        ? _session.createFood(brand: brand, name: name, kcalPerKg: kcalPerKg, bagGrams: bagGrams)
        : _session.updateFood(id: id, brand: brand, name: name, kcalPerKg: kcalPerKg, bagGrams: bagGrams),
  );

  /// Remove food `id`.
  Future<bool> deleteFood(String id) => change(() => _session.deleteFood(id: id));
}
