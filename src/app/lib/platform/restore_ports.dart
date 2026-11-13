import 'dart:io';
import 'dart:typed_data';

import 'package:file_selector/file_selector.dart';
import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/src/rust/api/alerts.dart' show AlertHold;
import 'package:wellfed/src/rust/api/install.dart' as install;
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/restore.dart' as rust;
import 'package:wellfed/src/rust/api/restore.dart';
import 'package:wellfed/src/rust/api/store.dart' show identityHex;

/// What a restore reaches outside Dart: the Rust run, NSD, the foreground
/// service, the kept backup and the file picker. Behind this, so every screen
/// is tested with no Rust and no phone.
abstract interface class RestorePorts {
  /// Whether this device may put the vendor's firmware back. A phone keeps
  /// its process alive through the install's foreground service, and a
  /// computer holds off sleep for the run.
  bool get restoresFeeder;

  /// The installer's own folder.
  Future<String> dataDir();

  /// The checked copy of stock kept for the feeder with `identity`.
  String? keptBackup(String dataDir, List<int> identity);

  /// Whether the file at `path` is a copy of stock this app can put back.
  BackupCheck checkBackup(String path);

  /// When the file at `path` was written.
  Future<DateTime?> backupDate(String path);

  /// A file the owner picks, as a path this app can read; `null` when they
  /// picked none.
  Future<String?> pickBackup();

  /// One run, which goes once.
  RestoreRun newRun(RestoreSpec spec);

  /// A feeder's identity, as lowercase hex.
  String hexOf(List<int> identity);

  /// Where the feeder `identity` (hex) answers.
  Future<String?> locate(String identity);

  /// What a read over a session says of the feeder at `address`.
  Future<RestoreSeen> readFeeder(List<int> identity, String address);

  /// Each sighting of a feeder advertising Bluetooth setup.
  Stream<void> setupHeard();

  /// Whether this phone's Bluetooth is on, now and as it changes.
  Stream<BluetoothPower> bluetooth();

  /// Ask Android to turn Bluetooth on.
  Future<void> turnOnBluetooth();

  /// Forget the restore's key for the feeder with `identity`.
  void forgetKey(String dataDir, List<int> identity);

  /// Keep this process alive with a notification while the firmware goes
  /// across, or a computer awake. `false` when Android or the computer
  /// refused.
  Future<bool> holdProcess(String text);

  /// Let this process go, once the run has ended.
  Future<void> releaseProcess();

  /// Each time a computer's hold gave way while a run still held: the run's
  /// screen then says to keep it awake. Nothing on a phone.
  Stream<void> holdLapsed();
}

/// [RestorePorts] on Android and the desktops, with this device's key.
class AppRestorePorts implements RestorePorts {
  /// Ports signed by `_key`.
  AppRestorePorts(this._key) : _install = AppInstallPorts(_key);

  final ClientKey _key;

  /// The install's folders, NSD and foreground service are a restore's too.
  final AppInstallPorts _install;

  @override
  bool get restoresFeeder => true;

  @override
  Future<String> dataDir() async => (await _install.paths()).data;

  @override
  String? keptBackup(String dataDir, List<int> identity) => install.keptBackup(dataDir: dataDir, identity: identity);

  @override
  BackupCheck checkBackup(String path) => rust.checkBackup(path: path);

  @override
  Future<DateTime?> backupDate(String path) async {
    final file = File(path);
    return file.existsSync() ? file.lastModifiedSync() : null;
  }

  @override
  Future<String?> pickBackup() async => (await openFile())?.path;

  @override
  RestoreRun newRun(RestoreSpec spec) => RestoreRun(key: _key, spec: spec);

  @override
  String hexOf(List<int> identity) => identityHex(identity: identity);

  @override
  Future<String?> locate(String identity) => _install.locate(identity);

  @override
  Future<RestoreSeen> readFeeder(List<int> identity, String address) =>
      rust.readRestoreFeeder(key: _key, identity: identity, address: address);

  @override
  Stream<void> setupHeard() => _install.setupHeard();

  @override
  Stream<BluetoothPower> bluetooth() => _install.bluetooth();

  @override
  Future<void> turnOnBluetooth() => _install.turnOnBluetooth();

  @override
  void forgetKey(String dataDir, List<int> identity) =>
      rust.forgetRestoreKey(dataDir: dataDir, identity: Uint8List.fromList(identity));

  @override
  Future<bool> holdProcess(String text) => _install.holdFor(AlertHold.restore, text);

  @override
  Future<void> releaseProcess() => _install.releaseFor(AlertHold.restore);

  @override
  Stream<void> holdLapsed() => _install.holdLapsed();
}
