import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/setup.dart';
import 'package:wellfed/state/wifi/networks.dart';

import 'fakes.dart';

/// A run the test drives: its updates are the test's, its answers recorded.
class FakeRun with FakeOpaque implements InstallRun {
  final updates = StreamController<InstallUpdate>();
  final answers = <int, InstallAnswer>{};
  final notifications = <List<int>>[];
  int lost = 0;
  int stops = 0;

  @override
  Stream<InstallUpdate> run() => updates.stream;

  @override
  void answer({required int id, required InstallAnswer answer}) => answers[id] = answer;

  @override
  void linkLost() => lost++;

  @override
  void notified({required List<int> bytes}) => notifications.add(bytes);

  @override
  void stop() => stops++;

  void event(InstallEvent event) => updates.add(InstallUpdate.event(event: event));

  void ask(int id, InstallAsk ask) => updates.add(InstallUpdate.ask(id: id, ask: ask));

  void end(InstallEnd end) => updates.add(InstallUpdate.ended(end: end));

  void stopped(StopCase stop, {Resume resume = Resume.nothing, String? detail}) =>
      end(InstallEnd.stopped(case_: stop, resume: resume, detail: detail ?? ''));
}

class FakeListener with FakeOpaque implements InstallListener {
  // ignore: close_sinks, the test ends with it
  final updates = StreamController<ListenUpdate>();
  int stops = 0;

  @override
  Stream<ListenUpdate> run() => updates.stream;

  @override
  void stop() => stops++;
}

class FakeLink implements InstallLink {
  final written = <List<List<int>>>[];
  int closes = 0;

  @override
  Future<void> write(List<List<int>> pieces) async => written.add(pieces);

  @override
  Future<void> close() async => closes++;
}

/// The install's outside world, with no Rust and no phone.
class FakeInstallPorts implements InstallPorts {
  FakeInstallPorts({
    this.image = const ImageCheck.ready(provisioning: ImageProvisioning.consumer, build: 'ab', version: 7),
    this.build = const BuildCheck.reviewable(),
    this.ip = '192.168.1.23',
    this.pendingInstall = PendingInstall.nothing,
  });

  /// False to stand in for a desktop.
  @override
  bool joinsFeederNetwork = true;

  ImageCheck image;
  BuildCheck build;
  String? ip;
  PendingInstall pendingInstall;
  bool holds = true;
  String? kept;
  bool allowed = true;
  int asked = 0;

  final specs = <InstallSpec>[];
  final runs = <FakeRun>[];
  final listeners = <FakeListener>[];
  final held = <String>[];
  int releases = 0;
  final shared = <String>[];
  // ignore: close_sinks, the test ends with it
  final heard = StreamController<void>.broadcast();
  final link = FakeLink();
  void Function(List<int>)? onBytes;
  void Function()? onLost;
  Object? linkFails;
  String? located = '192.168.1.40:4747';

  /// The latest run.
  FakeRun get run => runs.last;

  @override
  Future<InstallPaths> paths() async => const InstallPaths(store: 'store', image: 'image', data: 'data');

  @override
  Future<ImageCheck> prepareImage(InstallPaths paths) async => image;

  @override
  BuildCheck checkBuild() => build;

  @override
  PendingInstall pending(InstallPaths paths) => pendingInstall;

  int forgets = 0;
  bool openRequest = false;
  Object? forgetFails;

  @override
  void forgetInstall(InstallPaths paths) {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (forgetFails case final error?) throw error;
    forgets++;
    pendingInstall = PendingInstall.nothing;
  }

  @override
  bool requestOpen(InstallPaths paths) => openRequest;

  int ledgersSetAside = 0;
  final backupsSetAside = <(String, BackupProblem)>[];

  @override
  void setAsideLedgers(InstallPaths paths) {
    ledgersSetAside++;
    pendingInstall = PendingInstall.nothing;
  }

  @override
  void setAsideBackup(InstallPaths paths, String path, BackupProblem problem) => backupsSetAside.add((path, problem));

  @override
  String? keptBackup(InstallPaths paths, List<int> identity) => kept;

  @override
  Future<String?> phoneAddress() async => ip;

  /// Android's picker: throws this instead of joining.
  Object? joinFails;
  int joined = 0;
  int left = 0;

  /// The address after leaving, when it is not [ip].
  String? ipAfterLeaving;

  @override
  Future<void> joinFeederNetwork() async {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (joinFails case final error?) throw error;
    joined++;
  }

  @override
  Future<String?> leaveFeederNetwork() async {
    left++;
    return ipAfterLeaving ?? ip;
  }

  @override
  Future<String?> phoneModel() async => null;

  @override
  Future<String?> currentNetwork() async => null;

  @override
  Future<NetworkScan> scanNetworks() async => const NetworksUnread(ScanProblem.unavailable);

  @override
  Future<bool> askPermissions({required bool bluetooth}) async {
    asked++;
    return allowed;
  }

  @override
  InstallRun newRun(InstallSpec spec) {
    specs.add(spec);
    final run = FakeRun();
    runs.add(run);
    return run;
  }

  @override
  InstallListener newListener(InstallPaths paths) {
    final listener = FakeListener();
    listeners.add(listener);
    return listener;
  }

  @override
  Duration get listenWait => const Duration(minutes: 20);

  @override
  Future<InstallLink> openLink({
    required void Function(List<int> bytes) onBytes,
    required void Function() onLost,
  }) async {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (linkFails case final error?) throw error;
    linksOpened++;
    this.onBytes = onBytes;
    this.onLost = onLost;
    await linkGate?.future;
    return link;
  }

  /// While set, a link opens only once it completes: a scan and a connect
  /// still out.
  Completer<void>? linkGate;

  @override
  Stream<void> setupHeard() => heard.stream;

  /// This phone's Bluetooth, and its changes to each listener.
  BluetoothPower power = BluetoothPower.on;
  final _powerChanges = StreamController<BluetoothPower>.broadcast();
  int turnOnAsks = 0;
  int linksOpened = 0;

  /// Turn Bluetooth to `to`, as the owner would in Android.
  void setPower(BluetoothPower to) {
    power = to;
    _powerChanges.add(to);
  }

  @override
  Stream<BluetoothPower> bluetooth() => Stream.multi((listener) {
    listener.add(power);
    final StreamSubscription<BluetoothPower> changes = _powerChanges.stream.listen(listener.add);
    listener.onCancel = changes.cancel;
  });

  @override
  Future<void> turnOnBluetooth() async => turnOnAsks++;

  @override
  String hexOf(List<int> identity) => identity.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join();

  @override
  Future<String?> locate(String identity) async => located;

  @override
  Future<bool> holdProcess(String text) async {
    held.add(text);
    return holds;
  }

  @override
  Future<void> releaseProcess() async => releases++;

  @override
  Future<void> shareBackup(String path) async => shared.add(path);
}

/// A BLE setup session the test scripts.
class FakeDriver with FakeOpaque implements SetupDriver {
  FakeDriver(this.positions);

  final List<int> positions;

  /// What the next notification comes to.
  List<SetupStep> script = const [];

  @override
  Uint8List code() => Uint8List.fromList(positions);

  @override
  List<Uint8List> start() => [
    Uint8List.fromList([1]),
  ];

  @override
  List<SetupStep> notified({required List<int> bytes}) => script;

  /// The feeder that proved the code, once the last step went.
  Uint8List? provenFeeder;

  @override
  Uint8List? proven() => provenFeeder;
}
