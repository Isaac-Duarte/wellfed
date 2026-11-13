import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:file_selector/file_selector.dart' show getSaveLocation;
import 'package:flutter/foundation.dart' show FlutterError;
import 'package:flutter/services.dart' show ByteData, MethodChannel, rootBundle;
import 'package:flutter_reactive_ble/flutter_reactive_ble.dart';
import 'package:permission_handler/permission_handler.dart';
import 'package:share_plus/share_plus.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/platform/host.dart';
import 'package:wellfed/platform/phone.dart';
import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/src/rust/api/alerts.dart' show AlertHold;
import 'package:wellfed/src/rust/api/install.dart' as rust;
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/store.dart' show identityHex;
import 'package:wellfed/state/wifi/networks.dart';

/// Where the installer's files live on this phone.
class InstallPaths {
  /// These paths.
  const InstallPaths({required this.store, required this.image, required this.data});

  /// The app's own file.
  final String store;

  /// Where the APK's image is unpacked.
  final String image;

  /// The stock server's config and records, and the backups.
  final String data;
}

/// A connected feeder's control characteristic, as the install's run writes
/// it. Its notifications go to the callbacks [InstallPorts.openLink] took.
abstract interface class InstallLink {
  /// Write `pieces` to the feeder, in order.
  Future<void> write(List<List<int>> pieces);

  /// Hang up.
  Future<void> close();
}

/// What an install reaches outside Dart: the Rust run, the radio, NSD,
/// Android's permissions and foreground service, the APK's image and the
/// share sheet. Behind this, so every screen is tested with no Rust and no
/// phone.
abstract interface class InstallPorts {
  /// Whether this device can join the feeder's own Wi-Fi, which every
  /// install goes through. Only the Android app can.
  bool get joinsFeederNetwork;

  /// Where the install keeps its files.
  Future<InstallPaths> paths();

  /// Unpack the APK's image, and check it against this build's pin.
  Future<ImageCheck> prepareImage(InstallPaths paths);

  /// Whether this build may send an install.
  BuildCheck checkBuild();

  /// What an earlier run left on the feeder.
  PendingInstall pending(InstallPaths paths);

  /// Clear the install in progress. Keeps the backups and the stock ledgers.
  void forgetInstall(InstallPaths paths);

  /// Any feeder has an install request with no known outcome.
  bool requestOpen(InstallPaths paths);

  /// Rename the stock ledgers that do not parse. The owner accepted what that
  /// gives up.
  void setAsideLedgers(InstallPaths paths);

  /// Rename the kept backup at `path`. The owner accepted what that gives up.
  void setAsideBackup(InstallPaths paths, String path, BackupProblem problem);

  /// The checked copy of stock kept for the feeder with `identity`.
  String? keptBackup(InstallPaths paths, List<int> identity);

  /// This phone's address on the Wi-Fi, or `null` when it is on none.
  Future<String?> phoneAddress();

  /// Join the feeder's setup network, through Android's picker, so this
  /// process's sockets go over it. Throws when none was joined.
  Future<void> joinFeederNetwork();

  /// Leave the setup network: this phone's address once it is back on its
  /// Wi-Fi, or `null` if it does not come back in time.
  Future<String?> leaveFeederNetwork();

  /// Ask Android for what the install needs; whether Bluetooth was allowed.
  /// `bluetooth` is `false` for a bench image, which pairs over the LAN.
  Future<bool> askPermissions({required bool bluetooth});

  /// The phone's model, for a name.
  Future<String?> phoneModel();

  /// The Wi-Fi this phone is on, when Android will say.
  Future<String?> currentNetwork();

  /// The networks this phone hears, for the feeder's Wi-Fi.
  Future<NetworkScan> scanNetworks();

  /// One run, which goes once.
  InstallRun newRun(InstallSpec spec);

  /// A listener for a request whose end is not known.
  InstallListener newListener(InstallPaths paths);

  /// How long a listener listens.
  Duration get listenWait;

  /// Find the first feeder advertising Bluetooth setup and connect to it,
  /// notifications on. Throws when none is heard in time.
  Future<InstallLink> openLink({required void Function(List<int> bytes) onBytes, required void Function() onLost});

  /// Whether a feeder is advertising Bluetooth setup: the replacement is
  /// running, and waiting for its owner. Each sighting is an event.
  Stream<void> setupHeard();

  /// Whether this phone's Bluetooth is on, now and as it changes.
  Stream<BluetoothPower> bluetooth();

  /// Ask Android to turn Bluetooth on.
  Future<void> turnOnBluetooth();

  /// A feeder's identity, as lowercase hex.
  String hexOf(List<int> identity);

  /// Where the feeder `identity` (hex) answers.
  Future<String?> locate(String identity);

  /// Keep this process alive with a notification, for as long as a run or a
  /// listener needs stock's server on this phone. `false` when Android
  /// refused, so the app's being on screen is all that keeps it alive.
  Future<bool> holdProcess(String text);

  /// Let this process go, once the run has ended.
  Future<void> releaseProcess();

  /// Hand the kept stock backup at `path` to the owner, to save elsewhere.
  Future<void> shareBackup(String path);
}

/// [InstallPorts] on Android, with this phone's key.
class AppInstallPorts implements InstallPorts {
  /// Ports signed by `_key`.
  const AppInstallPorts(this._key);
  static const _wifi = MethodChannel('io.github.isaacduarte.wellfed/wifi');

  final ClientKey _key;

  @override
  bool get joinsFeederNetwork => !onDesktopHost;

  /// The image's files, as `make app-installer-input` fills them in.
  static const _imageAssets = ['review.json', 'build-manifest.json', 'image.bin', 'update.manifest'];

  static const _locateWait = Duration(seconds: 12);

  @override
  Future<InstallPaths> paths() async {
    final Directory support = await appDataDirectory();
    return InstallPaths(
      store: '${support.path}/wellfed.json',
      image: '${support.path}/install/image',
      data: '${support.path}/install/stock',
    );
  }

  @override
  Future<ImageCheck> prepareImage(InstallPaths paths) async {
    final folder = Directory(paths.image);
    // Whatever an earlier build unpacked is not this build's.
    if (folder.existsSync()) folder.deleteSync(recursive: true);
    folder.createSync(recursive: true);
    for (final String name in _imageAssets) {
      final ByteData data;
      try {
        data = await rootBundle.load('assets/installer/$name');
      } on FlutterError {
        return const ImageCheck.absent();
      }
      final Uint8List bytes = data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes);
      await File('${paths.image}/$name').writeAsBytes(bytes);
    }
    return checkImage(dir: paths.image);
  }

  @override
  BuildCheck checkBuild() => rust.checkBuild();

  @override
  PendingInstall pending(InstallPaths paths) => pendingInstall(dataDir: paths.data);

  @override
  void forgetInstall(InstallPaths paths) => rust.forgetInstall(dataDir: paths.data);

  @override
  bool requestOpen(InstallPaths paths) => installRequestOpen(dataDir: paths.data);

  @override
  void setAsideLedgers(InstallPaths paths) => rust.setAsideLedgers(dataDir: paths.data);

  @override
  void setAsideBackup(InstallPaths paths, String path, BackupProblem problem) =>
      rust.setAsideBackup(dataDir: paths.data, path: path, problem: problem);

  @override
  String? keptBackup(InstallPaths paths, List<int> identity) =>
      rust.keptBackup(dataDir: paths.data, identity: identity);

  @override
  Future<String?> phoneAddress() => _wifi.invokeMethod<String>('phoneAddress');

  @override
  Future<void> joinFeederNetwork() => _wifi.invokeMethod<void>('joinFeeder');

  @override
  Future<String?> leaveFeederNetwork() => _wifi.invokeMethod<String>('leaveFeeder');

  @override
  Future<String?> phoneModel() => const AppPhonePorts().model();

  @override
  Future<String?> currentNetwork() => const AppPhonePorts().currentNetwork();

  @override
  Future<NetworkScan> scanNetworks() => scanNearbyNetworks();

  @override
  Future<bool> askPermissions({required bool bluetooth}) async {
    final Map<Permission, PermissionStatus> granted = await [
      if (bluetooth) ...[Permission.bluetoothScan, Permission.bluetoothConnect],
      // Only to read the Wi-Fi's name for the prefill, and to show the
      // install's notification; the install goes on without either.
      Permission.locationWhenInUse,
      Permission.notification,
    ].request();
    return !bluetooth ||
        (granted[Permission.bluetoothScan]?.isGranted == true &&
            granted[Permission.bluetoothConnect]?.isGranted == true);
  }

  @override
  InstallRun newRun(InstallSpec spec) => InstallRun(key: _key, spec: spec);

  @override
  InstallListener newListener(InstallPaths paths) => InstallListener(dataDir: paths.data);

  @override
  Duration get listenWait => Duration(seconds: InstallListener.waitSeconds());

  @override
  Future<InstallLink> openLink({
    required void Function(List<int> bytes) onBytes,
    required void Function() onLost,
  }) async {
    final DiscoveredDevice device = await SetupLink.scan().first.timeout(SetupLink.findTimeout);
    final SetupLink link = await SetupLink.open(device);
    return _Link(link, link.notifications.listen(onBytes, onError: (Object _) => onLost(), onDone: onLost));
  }

  @override
  Stream<void> setupHeard() => SetupLink.scan().map((_) {});

  @override
  Stream<BluetoothPower> bluetooth() => SetupLink.power();

  @override
  Future<void> turnOnBluetooth() => SetupLink.askToTurnOn();

  @override
  String hexOf(List<int> identity) => identityHex(identity: identity);

  @override
  Future<String?> locate(String identity) => Discovery.locate(identity, _locateWait);

  @override
  Future<bool> holdProcess(String text) => holdFor(AlertHold.install, text);

  /// Keep this process up for `run`, saying `text` in the service's notice,
  /// and catch-ups off the feeders while it goes. Whether Android went along.
  Future<bool> holdFor(AlertHold run, String text) => runHolds.hold(run, text);

  @override
  Future<void> releaseProcess() => releaseFor(AlertHold.install);

  /// `run` is done with this process; another run's hold stays.
  Future<void> releaseFor(AlertHold run) => runHolds.release(run);

  @override
  Future<void> shareBackup(String path) async {
    await SharePlus.instance.share(
      ShareParams(
        files: [XFile(path, mimeType: 'application/octet-stream')],
        subject: "The feeder's original firmware",
      ),
    );
  }
}

class _Link implements InstallLink {
  _Link(this._link, this._notifications);

  final BleLink _link;
  final StreamSubscription<List<int>> _notifications;

  @override
  Future<void> write(List<List<int>> pieces) => _link.write(pieces);

  @override
  Future<void> close() async {
    await _notifications.cancel();
    await _link.close();
  }
}

/// The firmware backups this phone keeps: which feeder, and handing one on.
abstract interface class BackupPorts {
  /// Every backup this phone keeps, whether or not its feeder is still in
  /// the app's list.
  Future<List<KeptBackup>> kept();

  /// Hand the backup at `path` to wherever the owner keeps files: the share
  /// sheet on a phone, a save dialog on a computer.
  Future<void> share(String path);
}

/// The kept backups, by the paths the install keeps them under.
class AppBackupPorts implements BackupPorts {
  /// Ports signed by `key`; a computer saves through `save`.
  AppBackupPorts(ClientKey key, {this._save = const SaveBackup()}) : _install = AppInstallPorts(key);

  final AppInstallPorts _install;
  final SaveBackup _save;

  @override
  Future<List<KeptBackup>> kept() async => rust.keptBackups(dataDir: (await _install.paths()).data);

  @override
  Future<void> share(String path) => onDesktopHost ? _save(path) : _install.shareBackup(path);
}

/// A backup saved on a computer: a copy where the owner picks. A Mac's share
/// menu opens from a corner of the window and offers no folder to keep a file
/// in, so a computer asks where to save instead.
class SaveBackup {
  /// Asks with `pick`, the save dialog unless a test gives another.
  const SaveBackup({this.pick = _dialog});

  /// Where to save a file suggested as `name`; `null` when the owner cancels.
  final Future<String?> Function(String name) pick;

  static Future<String?> _dialog(String name) async => (await getSaveLocation(suggestedName: name))?.path;

  /// Copy the backup at `path` where the owner picks. Cancelling saves
  /// nothing; a copy that fails throws its [FileSystemException].
  Future<void> call(String path) async {
    final String? to = await pick(File(path).uri.pathSegments.last);
    if (to == null) return;
    await File(path).copy(to);
  }
}
