import 'package:flutter/foundation.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';

/// One pet on the owner's server: its weights, its intake and the feeders it
/// eats from.
@immutable
final class PetState {
  /// `pet`, nothing more read yet.
  const PetState({required this.pet, this.weights = const [], this.intake, this.busy = false, this.error});

  /// The pet, as the server last said.
  final ServerPet pet;

  /// Its weight log, newest first.
  final List<ServerWeight> weights;

  /// What it ate, day by day; `null` before it is read.
  final ServerPetIntake? intake;

  /// A change is out.
  final bool busy;

  /// What the last read or change ran into; `null` once one goes through.
  final ServerError? error;

  /// This state with the named parts changed.
  PetState copyWith({
    ServerPet? pet,
    List<ServerWeight>? weights,
    ServerPetIntake? intake,
    bool? busy,
    ServerError? Function()? error,
  }) => PetState(
    pet: pet ?? this.pet,
    weights: weights == null ? this.weights : List.unmodifiable(weights),
    intake: intake ?? this.intake,
    busy: busy ?? this.busy,
    error: error == null ? this.error : error(),
  );
}

/// One pet's screen. These are the server's records: none of it reaches a
/// feeder. A weight whose answer was lost is read back in Rust before it is
/// logged again, so a reading is never doubled.
class PetFlow extends ChangeNotifier {
  /// `pet` on `_session`.
  PetFlow(this._session, ServerPet pet) : _state = PetState(pet: pet);

  /// Weights read at a time.
  static const weightPage = 200;

  /// The days of intake shown.
  static const intakeDays = 14;

  final ServerSession _session;
  bool _disposed = false;

  /// What the screen shows.
  PetState get state => _state;
  PetState _state;

  String get _id => _state.pet.id;

  /// Read the weights and the intake.
  Future<void> load() async {
    try {
      final List<ServerWeight> weights = await _weights();
      final ServerPetIntake intake = await _session.petIntake(pet: _id, days: intakeDays);
      _set(_state.copyWith(weights: weights, intake: intake, error: () => null));
    } on ServerError catch (error) {
      _set(_state.copyWith(error: () => error));
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
    } while (after != null && !_disposed);
    return oldestFirst.reversed.toList();
  }

  /// Log `grams`, weighed now.
  Future<bool> addWeight(int grams) => _change(() => _session.addWeight(pet: _id, grams: grams));

  /// Remove reading `id`, logged by mistake.
  Future<bool> deleteWeight(String id) => _change(() => _session.deleteWeight(pet: _id, weight: id));

  /// The pet eats from server feeder `feeder`, or no longer does.
  Future<bool> eatsFrom(String feeder, {required bool eats}) async {
    if (_state.busy) return false;
    _set(_state.copyWith(busy: true));
    try {
      final ServerPet pet = eats
          ? await _session.assignFeeder(pet: _id, feeder: feeder)
          : await _session.unassignFeeder(pet: _id, feeder: feeder);
      _set(_state.copyWith(pet: pet, busy: false, error: () => null));
    } on ServerError catch (error) {
      _set(_state.copyWith(busy: false, error: () => error));
      return false;
    }
    await load();
    return true;
  }

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

  void _set(PetState next) {
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
