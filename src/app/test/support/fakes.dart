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
import 'package:wellfed/platform/restore_ports.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/setup.dart';
import 'package:wellfed/src/rust/api/update.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/wifi/networks.dart';
import 'package:wellfed/theme/theme.dart';

import 'fake_install.dart';
import 'fake_phone.dart';

export 'fake_book.dart';
export 'fake_phone.dart';

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

  /// The Wi-Fi this phone is on.
  String? network;

  /// What a scan finds: by default, the phone lists nothing, as with no plugin.
  NetworkScan scan = const NetworksUnread(ScanProblem.unavailable);

  @override
  Future<String?> currentNetwork() async => network;

  @override
  Future<NetworkScan> scanNetworks() async => scan;
  final answers = <(String, String)>[];

  @override
  Future<void> answered(String identity, String build) async => answers.add((identity, build));

  FakeSession get session => sessions.last;

  @override
  FeederSession open(String identity, String address, {required bool syncClock}) {
    final session = FakeSession();
    sessions.add(session);
    dialed.add(address);
    return session;
  }

  /// Where each session dialed, in order.
  final dialed = <String>[];

  @override
  Future<String?> locate(String identity, Duration wait) async {
    looks++;
    return 'lan:4747';
  }

  /// How many times NSD looked.
  int looks = 0;

  /// The bridge's rule, for the shapes the tests type: an IPv4 address, with
  /// the device API's port when it names none.
  @override
  String? parseAddress(String text) {
    final RegExpMatch? parts = RegExp(r'^(\d{1,3}(?:\.\d{1,3}){3})(?::(\d{1,5}))?$').firstMatch(text.trim());
    if (parts == null) return null;
    return '${parts[1]}:${parts[2] ?? 4747}';
  }

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
