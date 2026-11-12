import 'package:flutter/foundation.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/server/records.dart';

/// One pet on the owner's server: its weights, its intake and the feeders it
/// eats from.
@immutable
final class PetState implements RecordsState<PetState> {
  /// `pet`, nothing more read yet.
  const PetState({
    required this.pet,
    this.weights = const [],
    this.intake,
    this.busy = false,
    this.problem,
    this.removed = false,
  });

  /// The pet, as the server last said.
  final ServerPet pet;

  /// Its weight log, newest first.
  final List<ServerWeight> weights;

  /// What it ate, day by day; `null` before it is read.
  final ServerPetIntake? intake;

  @override
  final bool busy;

  @override
  final RecordProblem? problem;

  /// The server removed it: the screen closes.
  final bool removed;

  /// This state with the named parts changed.
  PetState copyWith({ServerPet? pet, List<ServerWeight>? weights, ServerPetIntake? intake, bool? removed}) => PetState(
    pet: pet ?? this.pet,
    weights: weights == null ? this.weights : List.unmodifiable(weights),
    intake: intake ?? this.intake,
    busy: busy,
    problem: problem,
    removed: removed ?? this.removed,
  );

  @override
  PetState ran({required bool busy, required RecordProblem? problem}) =>
      PetState(pet: pet, weights: weights, intake: intake, busy: busy, problem: problem, removed: removed);
}

/// One pet's screen: the pet itself, its weights and the feeders it eats
/// from, all the server's records.
class PetFlow extends RecordsFlow<PetState> {
  /// `pet` on `_session`.
  PetFlow(this._session, ServerPet pet) : super(PetState(pet: pet));

  /// Weights read at a time.
  static const weightPage = 200;

  /// The days of intake shown.
  static const intakeDays = 14;

  final ServerSession _session;

  String get _id => state.pet.id;

  @override
  Future<void> load() async {
    if (state.removed) return;
    try {
      final List<ServerWeight> weights = await _weights();
      final ServerPetIntake intake = await _session.petIntake(pet: _id, days: intakeDays);
      show(state.copyWith(weights: weights, intake: intake).ran(busy: state.busy, problem: null));
    } on ServerError catch (error) {
      readFailed(error);
    }
  }

  /// The server pages oldest first and the screen lists newest first, so
  /// every page is read.
  Future<List<ServerWeight>> _weights() async {
    final oldestFirst = <ServerWeight>[];
    String? after;
    do {
      final ServerWeightPage page = await _session.weights(pet: _id, after: after, limit: weightPage);
      oldestFirst.addAll(page.items);
      if (page.next == after || page.items.isEmpty) break;
      after = page.next;
    } while (after != null && !disposed);
    return oldestFirst.reversed.toList();
  }

  /// Change the pet's own fields to these. A birth date is `YYYY-MM-DD`.
  Future<bool> save({required String name, required Species species, String? birthDate, int? dailyKcal}) => change(
    () => _session.updatePet(id: _id, name: name, species: species, birthDate: birthDate, dailyKcal: dailyKcal),
    took: _took,
  );

  /// Remove the pet, with its weights.
  Future<bool> remove() => change(() => _session.deletePet(id: _id), took: (_) => show(state.copyWith(removed: true)));

  /// Log `grams`, weighed now.
  Future<bool> addWeight(int grams) => change(() => _session.addWeight(pet: _id, grams: grams));

  /// Remove reading `id`, logged by mistake.
  Future<bool> deleteWeight(String id) => change(() => _session.deleteWeight(pet: _id, weight: id));

  /// The pet eats from server feeder `feeder`, or no longer does.
  Future<bool> eatsFrom(String feeder, {required bool eats}) => change(
    () => eats ? _session.assignFeeder(pet: _id, feeder: feeder) : _session.unassignFeeder(pet: _id, feeder: feeder),
    took: _took,
  );

  void _took(ServerPet pet) => show(state.copyWith(pet: pet));
}
