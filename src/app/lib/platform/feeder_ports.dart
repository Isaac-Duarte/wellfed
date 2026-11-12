import 'dart:typed_data';

import 'package:file_selector/file_selector.dart';
import 'package:share_plus/share_plus.dart';
import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/platform/host.dart';
import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/platform/phone.dart';
import 'package:wellfed/platform/recorder.dart';
import 'package:wellfed/platform/restore_ports.dart';
import 'package:wellfed/src/rust/api/alerts.dart' show AlertHold;
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart' show UpdateInfo;
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/model.dart' show Skew;
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/router.dart' as router_api show feedDueSeconds, settledFeed;
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/src/rust/api/update.dart';
import 'package:wellfed/src/rust/api/update.dart' as update_api show unknownEnd, updateOffer;
import 'package:wellfed/state/wifi/networks.dart';

/// One running update: what it reports, and where it is told the feeder is.
abstract interface class UpdatePush {
  /// What the run reports, up to its end.
  Stream<UpdateProgress> get progress;

  /// Answer the run's ask `id` with where the feeder answers.
  void located(int id, String? address);
}

/// What a feeder's controller reaches outside Dart: the Rust session, the
/// path router and NSD. Behind this, so its screens are tested with no Rust
/// and no network.
abstract interface class FeederPorts {
  /// A session with the feeder `identity` (hex) at `address`, not yet run.
  FeederSession open(String identity, String address, {required bool syncClock});

  /// The one router every request and feed goes through; `null` when its
  /// pins could not be opened, and then nothing is sent at all.
  PathRouter? get router;

  /// A feed's end as `settlement` gives it, for a feed whose `Fed` event this
  /// phone has not read; `null` for any settlement but a feed's end.
  FeedResult? settledFeed(Settlement settlement);

  /// How long a feed of `portions` may take, in seconds, past which a feeder
  /// that stayed on has ended it.
  int feedDueSeconds(int portions);

  /// Adopt `feeder`, paired before the server, onto the server through the
  /// router: an owner's invite made with this phone's key, which becomes the
  /// account's phone key.
  Future<Adoption> adopt(KnownFeeder feeder);

  /// Read the server's list again after an adoption whose answer was lost.
  /// It sends nothing but the claim.
  Future<Adoption> checkAdoption(String identity);

  /// Claim this phone's key on `identity` as the account's, for an adoption
  /// that left it unclaimed.
  Future<Adoption> claimPhone(String identity);

  /// A session with this feeder read `build`, which settles the stock trial
  /// that installed it, if one is open.
  Future<void> answered(String identity, String build);

  /// Where the feeder answers, within `wait`.
  Future<String?> locate(String identity, Duration wait);

  /// `text` as the `ip:port` a session dials, or `null` when it is neither
  /// `ip` nor `ip:port`.
  String? parseAddress(String text);

  /// The image this APK carries, unpacked and checked against its pin.
  Future<ImageCheck> updateImage();

  /// Whether to offer the image `build` at `version` to a feeder that runs
  /// `running` at `skew`, with `update` its update state (`null` before it is
  /// read, or when its read failed): the rule every client shares, decided
  /// in Rust.
  UpdateOffer updateOffer({
    required String build,
    required int version,
    String? running,
    required Skew skew,
    UpdateInfo? update,
  });

  /// What a fresh update state and the feeder's current status settle of an
  /// update whose end was unknown, decided in Rust by the same rule.
  UnknownEnd unknownEnd({
    String? served,
    required int version,
    required UpdateInfo update,
    String? running,
    required bool ready,
  });

  /// Whether this device may update a feeder's firmware. Only the Android
  /// app has: the run keeps its process alive through the install's
  /// foreground service, which has no desktop side.
  bool get updatesFeeder;

  /// One update of the feeder to that image, which goes once.
  UpdatePush pushUpdate(String identity, String? address);

  /// Keep this process alive with a notification while the firmware goes
  /// across. `false` when Android refused.
  Future<bool> holdProcess(String text);

  /// Let this process go, once the firmware is across.
  Future<void> releaseProcess();

  /// What the restore screen reaches outside Dart.
  RestorePorts get restore;

  /// Hand `text` to Android's share sheet.
  Future<void> share(String text);

  /// Ask the owner for a WAV; null when they back out.
  Future<Uint8List?> pickRecording();

  /// The phone's microphone, for recording the meal call here.
  Recorder get recorder;

  /// The Wi-Fi this phone is on, when Android will say.
  Future<String?> currentNetwork();

  /// The networks this phone hears, for the feeder's Wi-Fi.
  Future<NetworkScan> scanNetworks();
}

/// [FeederPorts] on Android, with this phone's key.
class AppFeederPorts implements FeederPorts {
  /// Ports signed by `_key`, routed by `router`.
  AppFeederPorts(this._key, this.router);

  @override
  final PathRouter? router;

  @override
  bool get updatesFeeder => !onDesktopHost;

  @override
  FeedResult? settledFeed(Settlement settlement) => router_api.settledFeed(settlement: settlement);

  @override
  int feedDueSeconds(int portions) => router_api.feedDueSeconds(portions: portions);

  @override
  Future<Adoption> adopt(KnownFeeder feeder) =>
      _routed((router) => router.adopt(feeder: feeder.identity, name: feeder.name, address: feeder.address, key: _key));

  @override
  Future<Adoption> checkAdoption(String identity) =>
      _routed((router) => router.checkAdoption(feeder: identity, key: _key));

  @override
  Future<Adoption> claimPhone(String identity) => _routed((router) => router.claimPhone(feeder: identity, key: _key));

  /// With no router nothing goes to the server either.
  Future<Adoption> _routed(Future<Adoption> Function(PathRouter router) call) {
    final PathRouter? routing = router;
    if (routing == null) {
      return Future.value(
        const Adoption.failed(
          error: ServerError(
            kind: ServerErrorKind.closed,
            message: 'the pending writes could not be opened',
            mayHaveRun: false,
          ),
        ),
      );
    }
    return call(routing);
  }

  @override
  Future<void> share(String text) async {
    await SharePlus.instance.share(ShareParams(text: text));
  }

  @override
  late final Recorder recorder = AppRecorder();

  @override
  Future<String?> currentNetwork() => const AppPhonePorts().currentNetwork();

  @override
  Future<NetworkScan> scanNetworks() => scanNearbyNetworks();

  @override
  Future<Uint8List?> pickRecording() async {
    // Android's picker filters by MIME type, and turns the extension into just
    // `audio/x-wav`: a WAV a file app labels with any of the other names it
    // goes by was greyed out. The bridge checks the bytes either way.
    const wav = XTypeGroup(
      label: 'WAV',
      extensions: ['wav'],
      mimeTypes: ['audio/wav', 'audio/x-wav', 'audio/wave', 'audio/vnd.wave'],
    );
    final XFile? picked = await openFile(acceptedTypeGroups: const [wav]);
    return picked?.readAsBytes();
  }

  final ClientKey _key;

  /// The unpacking and paths the install uses: one image, one place.
  late final _install = AppInstallPorts(_key);

  @override
  Future<void> answered(String identity, String build) async {
    final InstallPaths paths = await _install.paths();
    feederAnswered(
      dataDir: paths.data,
      identity: identityBytes(hex: identity),
      build: build,
    );
  }

  @override
  FeederSession open(String identity, String address, {required bool syncClock}) => FeederSession(
    key: _key,
    feeder: identityBytes(hex: identity),
    address: address,
    syncClock: syncClock,
  );

  @override
  Future<String?> locate(String identity, Duration wait) => Discovery.locate(identity, wait);

  @override
  String? parseAddress(String text) {
    try {
      return feederAddress(text: text);
    } on BridgeError catch (error) {
      if (error.kind == ErrorKind.address) return null;
      rethrow;
    }
  }

  @override
  Future<ImageCheck> updateImage() async => _install.prepareImage(await _install.paths());

  @override
  UpdateOffer updateOffer({
    required String build,
    required int version,
    String? running,
    required Skew skew,
    UpdateInfo? update,
  }) => update_api.updateOffer(build: build, version: version, running: running, skew: skew, update: update);

  @override
  UnknownEnd unknownEnd({
    String? served,
    required int version,
    required UpdateInfo update,
    String? running,
    required bool ready,
  }) => update_api.unknownEnd(served: served, version: version, update: update, running: running, ready: ready);

  @override
  UpdatePush pushUpdate(String identity, String? address) => _AppPush(_key, _install, identity, address);

  @override
  Future<bool> holdProcess(String text) => _install.holdFor(AlertHold.update, text);

  @override
  Future<void> releaseProcess() => _install.releaseFor(AlertHold.update);

  @override
  late final RestorePorts restore = AppRestorePorts(_key);
}

class _AppPush implements UpdatePush {
  _AppPush(this._key, this._install, this._identity, this._address);

  final ClientKey _key;
  final AppInstallPorts _install;
  final String _identity;
  final String? _address;
  FirmwareUpdate? _run;

  @override
  Stream<UpdateProgress> get progress async* {
    final InstallPaths paths = await _install.paths();
    final run = FirmwareUpdate(
      key: _key,
      feeder: identityBytes(hex: _identity),
      address: _address,
      imageDir: paths.image,
    );
    _run = run;
    yield* run.run();
  }

  @override
  void located(int id, String? address) => _run?.answer(
    id: id,
    answer: InstallAnswer.located(address: address),
  );
}
