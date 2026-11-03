import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart' hide Card;
import 'package:flutter_reactive_ble/flutter_reactive_ble.dart' show DiscoveredDevice, Uuid;
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart' show RustOpaqueInterface;
import 'package:flutter_test/flutter_test.dart' show WidgetTester;
import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/platform/feeder_ports.dart';
import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/platform/pairing_ports.dart';
import 'package:wellfed/platform/phone.dart';
import 'package:wellfed/platform/recorder.dart';
import 'package:wellfed/platform/restore_ports.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/setup.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/src/rust/api/update.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/theme/theme.dart';

import 'fake_install.dart';

/// Pump past transitions. Not `pumpAndSettle`: a screen waiting on the feeder
/// shows a spinner, which never settles.
Future<void> settle(WidgetTester tester) async {
  for (var frame = 0; frame < 8; frame++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

Widget themed(Widget child, {Brightness brightness = Brightness.light}) =>
    MaterialApp(theme: wellfedTheme(brightness), debugShowCheckedModeBanner: false, home: child);

mixin FakeOpaque implements RustOpaqueInterface {
  @override
  void dispose() {}

  @override
  bool get isDisposed => false;
}

/// The feeders the app knows, in memory.
class FakeBook extends ChangeNotifier implements FeederBook {
  FakeBook({this.label = 'Pixel'});
  @override
  Appearance appearance = Appearance.system;

  @override
  Future<void> setAppearance(Appearance next) async {
    appearance = next;
    notifyListeners();
  }

  @override
  String label;

  @override
  List<KnownFeeder> feeders = [];

  final remembered = <(KnownFeeder, bool)>[];

  /// What the next save throws, as a full or read-only disk would.
  Object? saveFails;

  @override
  Future<void> setLabel(String label) async {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (saveFails case final error?) throw error;
    this.label = label;
    notifyListeners();
  }

  @override
  Future<void> remember(KnownFeeder feeder, {bool paired = false, bool keepName = false}) async {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (saveFails case final error?) throw error;
    remembered.add((feeder, paired));
    final Iterable<KnownFeeder> known = feeders.where((f) => f.identity == feeder.identity);
    feeders = [
      for (final other in feeders)
        if (other.identity != feeder.identity) other,
      KnownFeeder(
        identity: feeder.identity,
        name: keepName && known.isNotEmpty ? known.first.name : feeder.name,
        address: feeder.address,
      ),
    ];
    notifyListeners();
  }

  @override
  Future<void> answeredAt(String identity, String address) async {}

  @override
  Future<void> forget(String identity) async {
    feeders = [
      for (final feeder in feeders)
        if (feeder.identity != identity) feeder,
    ];
    notifyListeners();
  }

  /// Set to make the next save fail, as a full disk would.
  BridgeError? renameFails;

  @override
  Future<void> rename(String identity, String name) async {
    if (renameFails case final failure?) throw failure;
    feeders = [
      for (final feeder in feeders)
        if (feeder.identity == identity)
          KnownFeeder(identity: feeder.identity, name: name, address: feeder.address)
        else
          feeder,
    ];
    notifyListeners();
  }

  final _clockDue = <String>{};

  void clockOwed(String identity) => _clockDue.add(identity);

  @override
  bool clockDue(String identity) => _clockDue.contains(identity);

  @override
  void clockSynced(String identity) => _clockDue.remove(identity);
}

/// A recovery the test scripts, as its stream would.
class FakeRecovery with FakeOpaque implements Recovery {
  // ignore: close_sinks, the test ends with it
  final steps = StreamController<SetupStep>();
  int closes = 0;

  @override
  void close() => closes++;

  @override
  Uint8List code() => Uint8List.fromList(const [1, 2, 3, 4, 1, 2]);

  @override
  Stream<SetupStep> run({required String address}) => steps.stream;
}

class FakeBleLink implements BleLink {
  // ignore: the test ends with it
  // ignore: close_sinks, the test ends with it
  final bytes = StreamController<List<int>>(onCancel: () async {});
  final written = <List<List<int>>>[];
  int closes = 0;

  @override
  Stream<List<int>> get notifications => bytes.stream;

  @override
  Future<void> write(List<List<int>> pieces) async => written.add(pieces);

  @override
  Future<void> close() async => closes++;
}

DiscoveredDevice feederNearby(String id, {int rssi = -50}) => DiscoveredDevice(
  id: id,
  name: '',
  serviceData: const {},
  manufacturerData: Uint8List(0),
  rssi: rssi,
  serviceUuids: const <Uuid>[],
);

/// Pairing's outside world, with no Rust and no radio.
class FakePairingPorts implements PairingPorts {
  @override
  Stream<FoundFeeder> browse() => const Stream.empty();

  final drivers = <FakeDriver>[];
  final recoveries = <FakeRecovery>[];
  // ignore: the test ends with it
  // ignore: close_sinks, the test ends with it
  final scans = StreamController<DiscoveredDevice>(onCancel: () async {});
  final opened = <DiscoveredDevice>[];
  FakeBleLink link = FakeBleLink();
  String? located = '192.168.1.40:4747';
  int nextCode = 1;

  @override
  SetupDriver newDriver(String label, WifiCredentials? wifi) {
    final List<int> code = [nextCode, 2, 3, 4, 1, 2];
    nextCode++;
    final driver = FakeDriver(code);
    drivers.add(driver);
    return driver;
  }

  @override
  Recovery newRecovery(String label) {
    final recovery = FakeRecovery();
    recoveries.add(recovery);
    return recovery;
  }

  @override
  Stream<DiscoveredDevice> scan() => scans.stream;

  @override
  Future<BleLink> open(DiscoveredDevice device) async {
    opened.add(device);
    link = FakeBleLink();
    return link;
  }

  @override
  Future<String?> locate(String identity, Duration wait) async => located;

  @override
  String hexOf(List<int> identity) => identity.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join();
}

final doneStep = SetupStep.done(
  enrolled: Enrolled.new_,
  wifi: WifiSaved.saved,
  feeder: Uint8List.fromList(const [0xab, 0xcd]),
);

/// A device API session the test drives: its updates are the test's, and each
/// request is recorded and answered from [replies].
class FakeSession with FakeOpaque implements FeederSession {
  // ignore: the test ends with it
  final updates = StreamController<SessionUpdate>();
  final requests = <FeederRequest>[];
  final feeds = <int>[];

  /// What the next requests get, in order; `Done` once they run out.
  final replies = <FeederReply>[];

  /// What a request gets instead, for a session that hangs or dies mid-way.
  Future<FeederReply> Function(FeederRequest)? answering;
  int closes = 0;

  @override
  Stream<SessionUpdate> run() => updates.stream;

  @override
  Future<FeederReply> ask({required FeederRequest request}) async {
    requests.add(request);
    final Future<FeederReply> Function(FeederRequest)? answer = answering;
    if (answer != null) return answer(request);
    if (updates.isClosed) return const FeederReply.notSent();
    return replies.isEmpty ? const FeederReply.done() : replies.removeAt(0);
  }

  @override
  void feed({required int portions}) => feeds.add(portions);

  /// What a cancel throws, as a session already gone does.
  Object? cancelFails;
  final cancels = <int>[];

  @override
  void cancelFeed({required int feed}) {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (cancelFails case final error?) throw error;
    cancels.add(feed);
  }

  @override
  void refresh() {}

  @override
  void close() => closes++;

  void opened() => updates.add(const SessionUpdate.opened(build: 'test', role: ClientRole.owner));

  void status(Card card) => updates.add(SessionUpdate.status(card: card));

  void history(List<HistoryEntry> entries) => updates.add(SessionUpdate.history(entries: entries, caughtUp: true));
}

/// A phone with no backups.
class FakeBackupPorts implements BackupPorts {
  @override
  Future<String?> kept(String identity) async => null;

  @override
  Future<void> share(String path) async {}
}

/// A phone with everything granted, and no plugins to ask.
class FakePhonePorts implements PhonePorts {
  @override
  Future<bool> setupGranted() async => true;

  @override
  Future<bool> askForSetup() async => true;

  @override
  Future<bool> askForBluetooth() async => true;

  @override
  Future<void> openSettings() async {}

  @override
  Future<String?> currentNetwork() async => null;

  @override
  Future<String?> model() async => null;
}

/// The microphone: starts when [allowed], and stops with [recorded].
class FakeRecorder implements Recorder {
  bool allowed = true;
  Uint8List? recorded = Uint8List.fromList([1, 2, 3]);
  int starts = 0;
  int stops = 0;

  /// Holds `start`'s answer until completed, as a slow microphone would.
  Completer<void>? opening;

  @override
  Future<bool> start() async {
    starts++;
    await opening?.future;
    return allowed;
  }

  @override
  Future<Uint8List?> stop() async {
    stops++;
    return recorded;
  }
}

class FakeFeederPorts implements FeederPorts {
  final sessions = <FakeSession>[];

  @override
  final FakeRecorder recorder = FakeRecorder();

  /// What went to the share sheet.
  final shared = <String>[];

  @override
  Future<void> share(String text) async => shared.add(text);

  @override
  Future<Uint8List?> pickRecording() async => null;
  final answers = <(String, String)>[];

  @override
  Future<void> answered(String identity, String build) async => answers.add((identity, build));

  FakeSession get session => sessions.last;

  @override
  FeederSession open(String identity, String address, {required bool syncClock}) {
    final session = FakeSession();
    sessions.add(session);
    return session;
  }

  @override
  Future<String?> locate(String identity, Duration wait) async => 'lan:4747';

  /// For a test that leaves a screen and comes back to the same feeder.
  late FeederController controller;

  ImageCheck image = const ImageCheck.absent();
  final pushes = <String?>[];
  final located = <(int, String?)>[];
  int holds = 0;
  int releases = 0;
  bool holdGranted = true;
  // ignore: close_sinks, a test ends with its stream open, as a run does when its screen goes.
  final progress = StreamController<UpdateProgress>();

  @override
  Future<ImageCheck> updateImage() async => image;

  @override
  UpdatePush pushUpdate(String identity, String? address) {
    pushes.add(address);
    return _FakePush(this);
  }

  @override
  Future<bool> holdProcess(String text) async {
    holds++;
    return holdGranted;
  }

  @override
  Future<void> releaseProcess() async => releases++;

  @override
  RestorePorts get restore => throw UnimplementedError('no restore here');
}

class _FakePush implements UpdatePush {
  _FakePush(this._ports);

  final FakeFeederPorts _ports;

  @override
  Stream<UpdateProgress> get progress => _ports.progress.stream;

  @override
  void located(int id, String? address) => _ports.located.add((id, address));
}

/// A feeder that is ready, with what a test leaves out unread.
Card fakeCard({
  String build = 'test',
  FoodLevel food = FoodLevel.present,
  bool? locked = false,
  bool ready = true,
  Blocked? blocked,
  bool? adapter = true,
  int? batteryPercent = 90,
  bool clockSet = true,
  int rssi = -50,
  FaultView? fault,
  Desiccant desiccant = const Desiccant.remaining(days: 9, due: false),
  NextMealView? nextMeal,
  FeedResult? lastFed,
  List<MealView> meals = const [],
  TimeInfo? time,
  SettingsView? settings,
  PlansView? plans,
  int uptimeMs = 1000,
}) => Card(
  build: build,
  food: food,
  locked: locked,
  ready: ready,
  blocked: blocked,
  adapter: adapter,
  batteryPercent: batteryPercent,
  clockSet: clockSet,
  rssi: rssi,
  fault: fault,
  desiccant: desiccant,
  uptimeMs: uptimeMs,
  bank: 1,
  nextMeal: nextMeal,
  lastFed: lastFed,
  meals: meals,
  time: time,
  settings: settings,
  plans: plans,
  wifiGeneration: 1,
);

FeedResult fakeFed({
  int feed = 7,
  int requested = 2,
  int confirmed = 2,
  int? atMost,
  FeedOutcome outcome = FeedOutcome.done,
  FeedReason reason = FeedReason.done,
  int? atMs,
  FedBy by = const FedBy.panel(),
}) => FeedResult(
  feed: feed,
  requested: requested,
  confirmed: confirmed,
  atMost: atMost ?? confirmed,
  outcome: outcome,
  reason: reason,
  uncertain: outcome == FeedOutcome.uncertain,
  words: '$confirmed of $requested portions, ${outcome.name}',
  atMs: atMs,
  by: by,
  hopper: FoodLevel.present,
);

extension CopyBy on FeedResult {
  /// The same feed, asked for by a client.
  FeedResult copyByClient(String fingerprint) => FeedResult(
    feed: feed,
    requested: requested,
    confirmed: confirmed,
    atMost: atMost,
    outcome: outcome,
    reason: reason,
    uncertain: uncertain,
    words: words,
    atMs: atMs,
    by: FedBy.client(fingerprint: fingerprint),
    hopper: hopper,
  );
}
