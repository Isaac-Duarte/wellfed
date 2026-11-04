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
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
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
    final Process process = await Process.start('$target/wellfed-feeder-sim', [
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

/// The real session; the rest of what a feeder's screens reach is not in play.
class _Ports implements FeederPorts {
  _Ports(this._key);

  final ClientKey _key;

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
  UpdatePush pushUpdate(String identity, String? address) => throw UnimplementedError();

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

  setUpAll(() async {
    if (target == null) return;
    await RustLib.init(externalLibrary: ExternalLibrary.open('$target/libwellfed_app_bridge.so'));
    owner = ClientKey.generate();
    sim = await _Feeder.start(target, owner);
    final known = KnownFeeder(identity: sim.identity, name: 'Smoke', address: sim.address);
    feeder = FeederController(FakeBook()..feeders = [known], _Ports(owner), known);
    await _until(feeder, (state) => state.cardIsCurrent);
  });

  tearDownAll(() async {
    if (target == null) return;
    feeder.dispose();
    await sim.stop();
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
