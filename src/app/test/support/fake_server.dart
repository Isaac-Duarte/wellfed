import 'dart:async';

import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/platform/server_ports.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';

import 'fakes.dart';

const serverLink = ServerLink(
  url: 'http://home.lan:8080',
  user: 'ada',
  tokenId: '00000000-0000-0000-0000-000000000001',
);
const serverOwner = ServerAccount(id: 'u1', name: 'ada', role: ServerRole.owner);

ServerError serverError(ServerErrorKind kind) => ServerError(kind: kind, message: kind.name);

/// A session with the owner's server the test answers. Only what the app's
/// controllers call is here; anything else fails the test.
class FakeServerSession with FakeOpaque implements ServerSession {
  FakeServerSession({this.who = serverOwner});

  /// Who the token speaks for; a failure when set to one.
  ServerAccount who;
  ServerError? accountFails;
  ServerError? logoutFails;
  int logouts = 0;
  int closes = 0;

  /// Each feeder's status, by its server id.
  final details = <String, ServerFeederDetail>{};

  @override
  ServerLink link() => linked;

  /// Where it is, and as whom.
  ServerLink linked = serverLink;

  /// While set, the account read waits for it: a server slow to answer.
  Future<void>? accountWait;

  @override
  String secret() => 'token-secret';

  @override
  Future<ServerAccount> account() async {
    await accountWait;
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (accountFails case final error?) throw error;
    return who;
  }

  /// The password each logout carried.
  final logoutPasswords = <String>[];

  /// The key each logout named as the phone's own.
  final logoutKeys = <ClientKey>[];

  @override
  Future<void> logout({required String password, required ClientKey key}) async {
    logoutPasswords.add(password);
    logoutKeys.add(key);
    logouts++;
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (logoutFails case final error?) throw error;
  }

  @override
  Future<ServerFeederDetail> feeder({required String id}) async {
    final ServerFeederDetail? detail = details[id];
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (detail == null) throw serverError(ServerErrorKind.notFound);
    return detail;
  }

  @override
  void close() => closes++;

  /// The home's records, which the server screens read and change.
  final FakeHome home = FakeHome();

  @override
  Future<List<ServerPet>> pets() => home.read(() => home.pets);

  @override
  Future<List<ServerFood>> foods() => home.read(() => home.foods);

  @override
  Future<ServerPet> createPet({required String name, required Species species, String? birthDate, int? dailyKcal}) =>
      home.write('create pet $name', () {
        final pet = ServerPet(id: 'p${home.pets.length + 1}', name: name, species: species, feeders: const []);
        home.pets = [...home.pets, pet];
        return pet;
      });

  @override
  Future<ServerPet> updatePet({
    required String id,
    required String name,
    required Species species,
    String? birthDate,
    int? dailyKcal,
  }) => home.write('update pet $id $name', () {
    final pet = ServerPet(
      id: id,
      name: name,
      species: species,
      birthDate: birthDate,
      dailyKcal: dailyKcal,
      feeders: const [],
    );
    home.pets = home.pets.map((each) => each.id == id ? pet : each).toList();
    return pet;
  });

  @override
  Future<void> deletePet({required String id}) =>
      home.write('delete pet $id', () => home.pets = [...home.pets.where((pet) => pet.id != id)]);

  @override
  Future<ServerFood> createFood({
    required String brand,
    required String name,
    required int kcalPerKg,
    required int bagGrams,
  }) => home.write('create food $name', () {
    final food = ServerFood(
      id: 'f${home.foods.length + 1}',
      brand: brand,
      name: name,
      kcalPerKg: kcalPerKg,
      bagGrams: bagGrams,
    );
    home.foods = [...home.foods, food];
    return food;
  });

  @override
  Future<void> deleteFood({required String id}) =>
      home.write('delete food $id', () => home.foods = [...home.foods.where((food) => food.id != id)]);

  @override
  Future<ServerEventPage> events({required String feeder, int? epoch, BigInt? after, int? limit}) =>
      home.read(() => home.pages[after] ?? const ServerEventPage(entries: []));

  @override
  Future<ServerFeederFood> feederFood({required String feeder}) => home.read(() => home.food);

  @override
  Future<ServerRefillPage> refills({required String feeder, String? after, int? limit}) =>
      home.read(() => ServerRefillPage(items: home.refills));

  @override
  Future<ServerRefill> addRefill({required String feeder, required NewRefill refill}) =>
      home.write('refill $feeder ${refill is NewRefill_FullBag ? 'full' : 'grams'}', () {
        final made = ServerRefill(
          id: 'r${home.refills.length + 1}',
          feeder: feeder,
          atMs: 0,
          grams: switch (refill) {
            NewRefill_Grams(:final int grams) => grams,
            NewRefill_FullBag() => 2000,
          },
          fullBag: refill is NewRefill_FullBag,
          user: 'u1',
        );
        home.refills = [...home.refills, made];
        return made;
      });

  @override
  Future<ServerWeightPage> weights({required String pet, String? after, int? limit}) =>
      home.read(() => ServerWeightPage(items: home.weights));

  @override
  Future<ServerWeight> addWeight({required String pet, required int grams, int? atMs}) =>
      home.write('weigh $pet $grams', () {
        final made = ServerWeight(id: 'w${home.weights.length + 1}', atMs: 0, grams: grams);
        home.weights = [...home.weights, made];
        return made;
      });

  @override
  Future<ServerPetIntake> petIntake({required String pet, int? days}) => home.read(() => home.intake(pet));

  @override
  Future<ServerPet> assignFeeder({required String pet, required String feeder}) =>
      home.write('assign $pet $feeder', () => home.pets.firstWhere((each) => each.id == pet));

  // The controllers call none of the rest.
  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError('${invocation.memberName}');
}

/// Secure storage and the server's login, in memory.
class FakeServerPorts implements ServerPorts {
  FakeServerSession session = FakeServerSession();

  /// What the next login throws.
  ServerError? loginFails;

  /// Every login's arguments, the password among them, as the phone sent them.
  final logins = <({String url, String name, String password, String phone})>[];
  String? kept;
  int forgets = 0;

  /// What reading the kept token throws while set, as a Keystore fault
  /// would, the token still there.
  Object? secretFails;

  @override
  Future<ServerSession> login({
    required String url,
    required String name,
    required String password,
    required String phone,
  }) async {
    logins.add((url: url, name: name, password: password, phone: phone));
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (loginFails case final error?) throw error;
    return session;
  }

  /// What restoring a kept session throws while set.
  ServerError? restoreFails;

  @override
  ServerSession restore(ServerLink link, String secret) {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (restoreFails case final error?) throw error;
    return session;
  }

  @override
  Future<String?> secret() async {
    // ignore: only_throw_errors, this stands in for the platform's exception.
    if (secretFails case final error?) throw error;
    return kept;
  }

  @override
  Future<void> keepSecret(String secret) async => kept = secret;

  @override
  Future<void> forgetSecret() async {
    kept = null;
    forgets++;
  }

  /// The servers NSD finds.
  // ignore: close_sinks, the test ends with it
  final found = StreamController<FoundServer>.broadcast();

  @override
  Stream<FoundServer> browse() => found.stream;
}

/// The records of a [FakeServerSession]'s home, and every change asked of it.
class FakeHome {
  List<ServerPet> pets = const [];
  List<ServerFood> foods = const [];
  List<ServerRefill> refills = const [];
  List<ServerWeight> weights = const [];

  /// The pages of a feeder's history, by the `after` that reads each.
  Map<BigInt?, ServerEventPage> pages = const {};

  /// The feeder's food view; uncalibrated, with nothing since a refill.
  ServerFeederFood food = const ServerFeederFood(weight: PortionWeight.uncalibrated(), anomalies: []);

  /// The days of a pet's intake.
  List<ServerIntakeDay> days = const [];

  /// What every read and change throws while set.
  ServerError? fails;

  /// What the next change throws after it ran, its answer lost on the way.
  ServerError? lost;

  /// Every change asked, in order.
  final changes = <String>[];

  ServerPetIntake intake(String pet) =>
      ServerPetIntake(pet: pet, zone: 'UTC', feeders: const [], days: days, anomalies: const []);

  Future<T> read<T>(T Function() value) async {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (fails case final error?) throw error;
    return value();
  }

  Future<T> write<T>(String change, T Function() made) async {
    changes.add(change);
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (fails case final error?) throw error;
    final T result = made();
    final ServerError? gone = lost;
    lost = null;
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (gone != null) throw gone;
    return result;
  }
}

/// This phone's key, as the screens hand it on: the tests only check it is
/// the one passed.
class FakeClientKey with FakeOpaque implements ClientKey {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
