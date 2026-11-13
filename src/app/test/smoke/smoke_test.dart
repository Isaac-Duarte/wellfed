import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/feeder_ports.dart';
import 'package:wellfed/platform/join_ports.dart';
import 'package:wellfed/platform/recorder.dart';
import 'package:wellfed/platform/restore_ports.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/join.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/router.dart' as settled_feed show feedDueSeconds, settledFeed;
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/src/rust/api/update.dart';
import 'package:wellfed/src/rust/api/update.dart' as update_rules show unknownEnd, updateOffer;
import 'package:wellfed/src/rust/frb_generated.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/state/join/join.dart';
import 'package:wellfed/state/wifi/networks.dart';

import '../support/fakes.dart';

/// The directory `make check-app` built the bridge and the simulator into.
final String? _target = Platform.environment['WELLFED_SMOKE_TARGET'];

/// Longer than any answer the simulator gives, and a feed of one portion.
const _patience = Duration(seconds: 60);

/// A feeder process on a loopback port of its own, for one test run.
class _Feeder {
  _Feeder(this._process, this._state, this.address, this.identity);

  final Process _process;
  final Directory _state;
  final String address;
  final String identity;

  static Future<_Feeder> start(String target, ClientKey owner) async {
    final Directory state = await Directory.systemTemp.createTemp('wellfed-smoke');
    final keyFile = File('${state.path}/owner.json');
    await keyFile.writeAsString(
      jsonEncode({
        'version': 1,
        'secret': identityHex(identity: owner.secret()),
        'label': 'Smoke',
        'feeders': <String>[],
      }),
    );
    final Process process = await Process.start('$target/wellfed-feeder-sim${Platform.isWindows ? '.exe' : ''}', [
      '--state',
      '${state.path}/flash',
      '--listen',
      '127.0.0.1:0',
      '--enroll-owner',
      keyFile.path,
    ]);
    unawaited(process.stdout.drain<void>());
    final listening = Completer<String>();
    process.stderr.transform(utf8.decoder).transform(const LineSplitter()).listen((line) {
      final Match? found = RegExp(r'^Device API on ([^;]+);').firstMatch(line);
      if (found != null && !listening.isCompleted) listening.complete(found[1]);
    });
    final String address = await listening.future.timeout(_patience);
    final Object? file = jsonDecode(await _settled(keyFile));
    if (file case {'feeders': [{'identity': final String identity}, ...]}) {
      return _Feeder(process, state, address, identity);
    }
    throw StateError('the simulator recorded no feeder in ${keyFile.path}');
  }

  /// The simulator records the feeder in the key file just after it listens.
  static Future<String> _settled(File keyFile) async {
    for (var tries = 0; tries < 100; tries++) {
      final String text = await keyFile.readAsString();
      if (text.contains('identity')) return text;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    throw StateError('the simulator recorded no feeder');
  }

  Future<void> stop() async {
    _process.kill();
    await _process.exitCode;
    await _state.delete(recursive: true);
  }
}

/// The real session and the real router, so every request and feed goes
/// through the router as the app sends it; the rest of
/// what a feeder's screens reach is not in play.
class _Ports implements FeederPorts {
  _Ports(this._key, this.router);

  final ClientKey _key;

  @override
  final FeederRouter router;

  @override
  FeedResult? settledFeed(Settlement settlement) => settled_feed.settledFeed(settlement: settlement);

  @override
  int feedDueSeconds(int portions) => settled_feed.feedDueSeconds(portions: portions);

  @override
  FeederSession open(String identity, String address, {required bool syncClock}) => FeederSession(
    key: _key,
    feeder: identityBytes(hex: identity),
    address: address,
    syncClock: syncClock,
  );

  @override
  Future<void> answered(String identity, String build) async {}

  @override
  Future<String?> locate(String identity, Duration wait) async => null;

  @override
  String? parseAddress(String text) => throw UnimplementedError();

  @override
  Future<bool> holdProcess(String text) async => true;

  @override
  Future<void> releaseProcess() async {}

  @override
  Stream<void> holdLapsed() => const Stream.empty();

  @override
  Future<void> share(String text) async {}

  @override
  Future<Uint8List?> pickRecording() async => null;

  /// The smoke flows record nothing.
  @override
  Recorder get recorder => throw UnsupportedError('no microphone in the smoke tests');

  @override
  Future<String?> currentNetwork() async => null;

  @override
  Future<NetworkScan> scanNetworks() async => const NetworksUnread(ScanProblem.unavailable);

  @override
  Future<ImageCheck> updateImage() => throw UnimplementedError();

  @override
  UpdateOffer updateOffer({
    required String build,
    required int version,
    String? running,
    required Skew skew,
    UpdateInfo? update,
  }) => update_rules.updateOffer(build: build, version: version, running: running, skew: skew, update: update);

  @override
  UnknownEnd unknownEnd({
    String? served,
    required int version,
    required UpdateInfo update,
    String? running,
    required bool ready,
  }) => update_rules.unknownEnd(served: served, version: version, update: update, running: running, ready: ready);

  @override
  UpdatePush pushUpdate(String identity, String? address) => throw UnimplementedError();

  @override
  bool get updatesFeeder => true;

  @override
  RestorePorts get restore => throw UnimplementedError();
}

/// The real invite and join; NSD is not in play, as the feeder's address is known.
class _Join implements JoinPorts {
  _Join(this._key, this._address);

  final ClientKey _key;
  final String _address;

  @override
  String? inviteFeeder(String text) => AppJoinPorts(_key).inviteFeeder(text);

  @override
  Future<JoinAnswer> join(String invite, String label, String address) =>
      AppJoinPorts(_key).join(invite, label, address);

  @override
  Future<String?> locate(String identity, Duration wait) async => _address;
}

/// Wait for `feeder` to reach what `done` looks for.
Future<void> _until(FeederController feeder, bool Function(FeederState) done) async {
  final reached = Completer<void>();
  void check() {
    if (done(feeder.state) && !reached.isCompleted) reached.complete();
  }

  feeder.addListener(check);
  check();
  await reached.future.timeout(_patience);
  feeder.removeListener(check);
}

void main() {
  final String? target = _target;
  final bool absent = target == null;
  late ClientKey owner;
  late _Feeder sim;
  late FeederController feeder;
  late Directory pins;

  setUpAll(() async {
    if (target == null) return;
    // Cargo names the bridge's shared library by the host's convention.
    final String library = Platform.isMacOS
        ? 'libwellfed_app_bridge.dylib'
        : Platform.isWindows
        ? 'wellfed_app_bridge.dll'
        : 'libwellfed_app_bridge.so';
    await RustLib.init(externalLibrary: ExternalLibrary.open('$target/$library'));
    owner = ClientKey.generate();
    sim = await _Feeder.start(target, owner);
    final known = KnownFeeder(identity: sim.identity, name: 'Smoke', address: sim.address);
    pins = await Directory.systemTemp.createTemp('wellfed-smoke-pins');
    final router = FeederRouter(pinsPath: '${pins.path}/pending.json');
    feeder = FeederController(FakeBook()..feeders = [known], _Ports(owner, router), known);
    await _until(feeder, (state) => state.cardIsCurrent);
  });

  tearDownAll(() async {
    if (target == null) return;
    feeder.dispose();
    await sim.stop();
    await pins.delete(recursive: true);
  });

  // The screens' tests decide offers with a Dart copy of the rules; it must
  // answer as the app's Rust does, case by case.
  test("the update rules the screens are tested with are the app's", skip: absent, () {
    final real = _Ports(owner, FeederRouter(pinsPath: '${pins.path}/rules.json'));
    final fake = FakeFeederPorts();
    UpdateInfo state({int bank = 1, int floor = 4, Bank0Check bank0 = const Bank0Check.match(), bool idle = true}) =>
        UpdateInfo(
          phase: idle ? UpdatePhase.idle : UpdatePhase.receiving,
          bank: bank,
          minVersion: floor,
          version: 0,
          imageLen: 0,
          written: 0,
          bank0: bank0,
        );
    final List<UpdateInfo> states = [
      for (final int bank in [0, 1])
        for (final int floor in [4, 5, 6])
          for (final Bank0Check bank0 in const [
            Bank0Check.unchecked(),
            Bank0Check.match(),
            Bank0Check.blocked(Bank0Blocked.unrecorded),
            Bank0Check.unknown(),
          ])
            for (final bool idle in [true, false]) state(bank: bank, floor: floor, bank0: bank0, idle: idle),
    ];
    // An update state not read yet, or whose read failed.
    for (final String? running in [null, 'BB22', 'aa11']) {
      for (final Skew skew in Skew.values) {
        expect(
          fake.updateOffer(build: 'bb22', version: 5, running: running, skew: skew),
          real.updateOffer(build: 'bb22', version: 5, running: running, skew: skew),
          reason: 'unread $running $skew',
        );
      }
    }
    for (final UpdateInfo update in states) {
      for (final String? running in [null, 'BB22', 'aa11']) {
        for (final Skew skew in Skew.values) {
          expect(
            fake.updateOffer(build: 'bb22', version: 5, running: running, skew: skew, update: update),
            real.updateOffer(build: 'bb22', version: 5, running: running, skew: skew, update: update),
            reason: '$update $running $skew',
          );
        }
        for (final String? served in [null, 'bb22']) {
          for (final bool ready in [false, true]) {
            expect(
              fake.unknownEnd(served: served, version: 5, update: update, running: running, ready: ready),
              real.unknownEnd(served: served, version: 5, update: update, running: running, ready: ready),
              reason: '$update $served $running $ready',
            );
          }
        }
      }
    }
  });

  test('a session opens and reads the card', skip: absent, () {
    expect(feeder.state.connection, Connection.online);
    expect(feeder.state.role, ClientRole.owner);
    expect(feeder.state.card, isNotNull);
  });

  test('settings read back through the bridge', skip: absent, () async {
    expect(await feeder.ask(const FeederRequest.getSettings()), isA<FeederReply_Settings>());
  });

  test('a meal added here reads back from the feeder', skip: absent, () async {
    final FeederReply before = await feeder.ask(const FeederRequest.getPlans());
    if (before is! FeederReply_Plans) fail('no plans: $before');
    const meal = PlanView(minute: 480, portions: 2, days: Weekday.values, enabled: true, call: 0);

    final FeederReply added = await feeder.ask(
      FeederRequest.editPlans(
        plans: before.plans,
        edit: const PlanEdit.add(plan: meal),
      ),
    );
    expect(added, isA<FeederReply_Done>());

    final FeederReply after = await feeder.ask(const FeederRequest.getPlans());
    if (after is! FeederReply_Plans) fail('no plans: $after');
    expect(after.plans.plans.nonNulls.map((plan) => plan.minute), contains(480));
  });

  test('a meal skipped here shows as skipped, and its undo brings it back', skip: absent, () async {
    // A few minutes ahead, so the meal is still to come all through the test.
    final DateTime now = DateTime.now();
    final int minute = now.hour * 60 + now.minute + 3;
    if (minute >= 24 * 60) return markTestSkipped('no minute is left today for a meal to skip');
    final FeederReply before = await feeder.ask(const FeederRequest.getPlans());
    if (before is! FeederReply_Plans) fail('no plans: $before');
    final plan = PlanView(minute: minute, portions: 1, days: Weekday.values, enabled: true, call: 0);
    final FeederReply added = await feeder.ask(
      FeederRequest.editPlans(
        plans: before.plans,
        edit: PlanEdit.add(plan: plan),
      ),
    );
    expect(added, isA<FeederReply_Done>());

    MealView? meal(FeederState state) => state.card?.meals.where((meal) => meal.minute == minute).firstOrNull;
    await _until(feeder, (state) => meal(state)?.action is MealAction_Skip);
    final MealView? offered = meal(feeder.state);
    if (offered == null) fail('the meal left the card');
    final MealAction? skip = offered.action;
    if (skip is! MealAction_Skip) fail('no skip offered: $skip');
    final FeederReply skipped = await feeder.ask(FeederRequest.skipMeal(slot: offered.slot, dueMs: skip.dueMs));
    expect(skipped, isA<FeederReply_Done>());

    // The skip's event has the session read the plans again.
    await _until(feeder, (state) => meal(state)?.standing is MealStanding_Skipped);
    final MealAction? undo = meal(feeder.state)?.action;
    if (undo is! MealAction_Undo) fail('no undo offered: $undo');
    expect(undo.dueMs, skip.dueMs, reason: 'the same meal');
    final FeederReply undone = await feeder.ask(FeederRequest.unskipMeal(slot: offered.slot, dueMs: undo.dueMs));
    expect(undone, isA<FeederReply_Done>());
    await _until(feeder, (state) => meal(state)?.action is MealAction_Skip);
  });

  test('a feed goes out once and comes back as its end', skip: absent, () async {
    expect(feeder.state.canFeed, isTrue, reason: 'card: ${feeder.state.card?.blocked}');
    feeder.feedNow(1);
    await _until(feeder, (state) => state.feed is FeedEnded || state.feed is FeedRefused);

    final FeedState end = feeder.state.feed;
    expect(end, isA<FeedEnded>());
    if (end is FeedEnded) expect(end.result.requested, 1);
    feeder.dismissFeed();
  });

  test('an invite made here lets a second phone join', skip: absent, () async {
    final FeederReply reply = await feeder.ask(const FeederRequest.createInvite(role: ClientRole.member));
    if (reply is! FeederReply_Invite) fail('no invite: $reply');

    final book = FakeBook();
    final flow = JoinFlow(book, _Join(ClientKey.generate(), sim.address))..seen(reply.invite.text);
    addTearDown(flow.dispose);
    await flow.find();
    await flow.join(label: 'Second', feederName: 'Smoke');

    expect(flow.state, isA<JoinJoined>().having((state) => state.role, 'role', ClientRole.member));
    expect(book.feeders.single.identity, sim.identity);
    await feeder.loadClients();
    expect(feeder.state.clients.map((client) => client.label), contains('Second'));
  });
}
