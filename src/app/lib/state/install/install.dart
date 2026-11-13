import 'dart:async';

import 'package:clock/clock.dart';
import 'package:flutter/foundation.dart';

import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/platform/setup_link.dart' show BluetoothPower, bluetoothNow;
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/setup.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/form_factor.dart';
import 'package:wellfed/state/install/install_state.dart';
import 'package:wellfed/state/wifi/networks.dart';

export 'package:wellfed/state/install/install_state.dart';

part 'install_listen.dart';
part 'install_run.dart';
part 'install_steps.dart';

/// One install, from the first screen to its end.
class InstallFlow extends ChangeNotifier {
  /// An install saved to `_book`.
  InstallFlow(this._book, this._ports);

  /// What this phone calls the feeder until the owner names it.
  static const defaultFeederName = 'Feeder';

  final FeederBook _book;
  final InstallPorts _ports;

  /// How the install stands.
  InstallState get state => _state;
  InstallState _state = const InstallLoading(InstallFacts());

  InstallStage _stage = InstallStage.loading;

  ImageCheck? _image;
  BuildCheck? _build;

  /// This listen is for a request an earlier run left, not one this run sent.
  bool _listeningForEarlier = false;

  /// What an earlier run left on the feeder.
  PendingInstall _pending = PendingInstall.nothing;
  InstallPaths? _paths;

  // The handover.
  Handover _handover = Handover.starting;
  int? _joinAsk;

  /// Android's picker ended with no network: the feeder's network is not up
  /// yet, or the owner cancelled.
  bool _joinFailed = false;

  /// The owner said they held Reset, so the feeder needs no second hold.
  bool _feederReset = false;

  /// This phone's broker heard the feeder after a frame: stock kept it, and
  /// has no setup network left.
  bool _frameTaken = false;
  String? _frameAddress;

  String? _phoneIp;

  WaitReason? _waitReason;

  /// Stock connected but its reports were lost: restart it once more.
  bool _restartAgain = false;

  // The feeder check.
  String? _device;
  String? _deviceAddress;
  int? _recognizeAsk;

  // Sending.
  int _percent = 0;
  bool _restarting = false;
  DateTime? _sendingSince;

  // The claim.
  /// A bench image's recovery window, waiting for the owner.
  int? _windowAsk;

  /// A bench image's recovery window is waiting for the owner.
  bool get windowAsked => _windowAsk != null;
  List<int> _code = const [];
  DateTime? _deadline;
  bool _pressDispense = false;

  // The checklist.
  /// The feeder restarting onto the owner's Wi-Fi, after Bluetooth setup.
  CheckState _joining = CheckState.waiting;
  CheckState _copying = CheckState.waiting;
  double _copied = 0;
  CheckState _checked = CheckState.waiting;
  CheckState _promoting = CheckState.waiting;
  double _written = 0;

  /// A promotion of this install's may have gone in: a run stopped
  /// unsure of it. Until a clean start, what the feeder holds in place of
  /// its original firmware is not known here, and no stop says it does.
  bool _promotionMayHaveGone = false;
  CheckState _confirming = CheckState.waiting;

  /// A checked copy of stock.
  String? _backupPath;

  // The end.
  String? _identity;
  List<int>? _identityBytes;
  String? _address;
  InstallEnd_Stopped? _stop;

  // After an uncertain request.
  Listening _listening = Listening.idle;
  Duration _listened = Duration.zero;
  bool _stockSeen = false;

  /// A feeder nearby advertises setup while this listens. It says nothing about
  /// which feeder, so only the owner can say it is this one.
  bool _setupSeen = false;
  String? _listenerDetail;

  WifiCredentials? _home;
  bool _forget = false;

  /// The known feeder the owner picked for a run from bank 0. With none, a known
  /// feeder that answers is left alone.
  String? _pickedFeeder;

  /// The feeders this phone knows, for the owner to pick from.
  List<KnownFeeder> get knownFeeders => _book.feeders;

  /// The owner gave a new Wi-Fi for a feeder that never joined: the run goes
  /// through Bluetooth setup again, and the claim screen says how to get the
  /// feeder there.
  bool _settingUpAgain = false;

  /// The owner keeps waiting for the feeder this flow paired, which never
  /// joined. With no record to name it, it goes to the run as the pick, and the
  /// run waits for it to join rather than stopping because it does not answer.
  bool _keepWaiting = false;

  /// Android refused Bluetooth to a pick-up that may need it.
  bool _bluetoothRefused = false;

  /// This phone's Bluetooth was off when the run looked for the feeder over it.
  /// The run looks again on its own, so turning it on is all the owner does.
  bool _bluetoothOff = false;
  StreamSubscription<BluetoothPower>? _power;
  bool _permitted = false;

  InstallRun? _run;
  StreamSubscription<InstallUpdate>? _updates;
  InstallLink? _link;
  InstallListener? _listener;
  // ignore: cancel_subscriptions, _stopListening cancels it, in install_listen.dart.
  StreamSubscription<ListenUpdate>? _listenUpdates;
  // ignore: cancel_subscriptions, _stopListening cancels it, in install_listen.dart.
  StreamSubscription<void>? _heard;
  bool _holding = false;
  Timer? _holdHintTimer;

  /// This process's sockets go over the feeder's setup network.
  bool _onFeederNetwork = false;

  /// Android refused the foreground service, so only staying on screen keeps
  /// the broker alive.
  bool _holdRefused = false;

  /// A pick-up asked for the phone's name or the Wi-Fi first: it goes on once
  /// it has them.
  bool _resumeAfterWifi = false;
  String? _resumeAddress;
  bool _disposed = false;

  /// How long a listener listens, for "3 of 20 min".
  Duration get listenWait => _ports.listenWait;

  /// How the APK's image pairs, once it is known to be ready.
  ImageProvisioning? get provisioning => switch (_image) {
    ImageCheck_Ready(:final ImageProvisioning provisioning) => provisioning,
    _ => null,
  };

  /// A run is going: leaving means stopping it.
  bool get running => _run != null;

  /// Why this install cannot start, in the order the owner can act on.
  bool get mayStart => _image is ImageCheck_Ready && _build is BuildCheck_Reviewable;

  /// The run looks for a feeder advertising Bluetooth setup, rather than for
  /// one already on the owner's Wi-Fi.
  bool _searchingBluetooth = false;

  /// Bluetooth has found nothing for [setupHintAfter]: a feeder that is not
  /// advertising by itself takes the owner's hold.
  bool _setupHint = false;

  /// How long Bluetooth may find nothing before the owner is told to hold the button.
  static const setupHintAfter = Duration(seconds: 20);

  /// What the run waits for now, when nothing on the screen says so already:
  /// for a long wait, so a screen is never just still.
  String? get waitWords => switch (_stage) {
    InstallStage.claim when !_searchingBluetooth && _code.isEmpty && !windowAsked =>
      "Still looking for the feeder on your Wi-Fi. It's found once it has "
          'restarted and joined, which can take a few minutes.',
    InstallStage.sending when _restarting =>
      'Waiting for the feeder to restart into Wellfed and come back. This can '
          'take a few minutes.',
    InstallStage.sending when _percent >= 100 =>
      'The feeder has the firmware and is installing it. It restarts on its '
          'own, which can take a few minutes.',
    InstallStage.sending => 'Waiting for the feeder to ask for the next part of the firmware.',
    InstallStage.finishing when _joining == CheckState.running =>
      'Waiting for the feeder to restart and join your Wi-Fi. This can take a '
          'few minutes.',
    InstallStage.finishing when !_pressDispense => 'Waiting for the feeder to answer on your Wi-Fi.',
    _ => null,
  };

  /// A rough time left in stock's download, from how fast it has gone.
  Duration? get timeLeft {
    final DateTime? since = _sendingSince;
    if (since == null || _percent < 5 || _percent >= 100) return null;
    final Duration elapsed = DateTime.now().difference(since);
    return elapsed * ((100 - _percent) / _percent);
  }

  void _searchBluetooth() {
    _searchingBluetooth = true;
    if (_setupHint || _holdHintTimer != null) return;
    _holdHintTimer = Timer(setupHintAfter, () {
      _setupHint = true;
      _notify();
    });
  }

  void _foundOverBluetooth() {
    _holdHintTimer?.cancel();
    _holdHintTimer = null;
    _setupHint = false;
  }

  /// Read what decides whether this phone can install at all. A device that
  /// cannot join the feeder's Wi-Fi is refused first: every way through an
  /// install goes over that network.
  Future<void> load() async {
    if (!_ports.joinsFeederNetwork) return _go(InstallStage.unavailable);
    final InstallPaths paths = await _ports.paths();
    _paths = paths;
    _image = await _ports.prepareImage(paths);
    _build = _ports.checkBuild();
    _pending = _ports.pending(paths);
    _phoneIp = await _ports.phoneAddress();
    _go(InstallStage.before);
  }

  /// On from the first screen: the phone's name if it has none, then the
  /// Wi-Fi, then the Reset hold.
  Future<bool> begin() async {
    if (!await _permissions()) return false;
    _phoneIp = await _ports.phoneAddress();
    _nextAfter(InstallStage.before);
    return true;
  }

  /// What the install needs from Android. Every way into a run or a listener
  /// comes through here, since a fresh launch can take any of them first.
  Future<bool> _permissions() async {
    if (_permitted) return true;
    final bluetooth = provisioning == ImageProvisioning.consumer;
    _permitted = await _ports.askPermissions(bluetooth: bluetooth);
    _bluetoothRefused = !_permitted;
    return _permitted;
  }

  /// Save this phone's name and go on; why it could not be saved, or null.
  Future<String?> setLabel(String label) async {
    try {
      await _book.setLabel(label);
    } on BridgeError catch (error) {
      return error.message;
    }
    if (_resumeAfterWifi) {
      _resumeAfterWifi = false;
      unawaited(resume(address: _resumeAddress));
      return null;
    }
    _nextAfter(InstallStage.naming);
    return null;
  }

  /// How feeders list this phone so far.
  String get label => _book.label;

  /// The phone's model, for a name.
  Future<String?> phoneModel() => _ports.phoneModel();

  /// The Wi-Fi this phone is on, when Android will say.
  Future<String?> currentNetwork() => _ports.currentNetwork();

  /// The networks this phone hears, for the feeder's Wi-Fi.
  Future<NetworkScan> scanNetworks() => _ports.scanNetworks();

  /// Take the Wi-Fi for Bluetooth setup, or null to keep the feeder's own.
  void setHome(WifiCredentials? home) {
    _home = home;
    if (_resumeAfterWifi) {
      _resumeAfterWifi = false;
      unawaited(resume(address: _resumeAddress));
      return;
    }
    if (_feederReset) {
      unawaited(startHandover());
      return;
    }
    _nextAfter(InstallStage.wifi);
  }

  /// Back out of a screen. A pick-up that asked for it ends here: Continue from
  /// the first screen is the owner's choice of a new request.
  void back(InstallStage to) {
    _resumeAfterWifi = false;
    _pickedFeeder = null;
    _resumeAddress = null;
    _settingUpAgain = false;
    _keepWaiting = false;
    final InstallPaths? paths = _paths;
    if (to == InstallStage.before && paths != null) {
      _pending = _ports.pending(paths);
    }
    _go(to);
  }

  void _nextAfter(InstallStage from) {
    final InstallStage next = switch (from) {
      InstallStage.before when _book.label.isEmpty => InstallStage.naming,
      InstallStage.before || InstallStage.naming => InstallStage.wifi,
      _ => InstallStage.reset,
    };
    _go(next);
  }

  /// Why the last repair of a record failed, when it did.
  String? _repairFailed;

  /// Ask Android to turn this phone's Bluetooth on.
  Future<void> turnOnBluetooth() => _ports.turnOnBluetooth();

  /// End the run through its cleanup; the end says what it left.
  void stopRun() => _run?.stop();

  /// The feeder's name on this phone, and where it last answered.
  Future<void> finish(String name) async {
    final String? identity = _identity;
    if (identity == null) return;
    await _book.remember(KnownFeeder(identity: identity, name: name, address: _address));
  }

  /// Hand the kept backup to wherever the owner keeps files; why that failed, or null.
  Future<String?> saveBackup() async {
    final String? path = _backupPath;
    if (path == null) return null;
    try {
      await _ports.shareBackup(path);
      return null;
    } on Object catch (error) {
      return '$error';
    }
  }

  void _go(InstallStage next) {
    _stage = next;
    _notify();
  }

  void _notify() {
    _state = _snapshot();
    if (!_disposed) notifyListeners();
  }

  InstallState _snapshot() {
    final facts = InstallFacts(
      pending: _pending,
      bluetoothRefused: _bluetoothRefused,
      bluetoothOff: _bluetoothOff,
      repairFailed: _repairFailed,
      phoneIp: _phoneIp,
    );
    return switch (_stage) {
      InstallStage.loading => InstallLoading(facts),
      InstallStage.unavailable => InstallUnavailable(facts),
      InstallStage.before => InstallBefore(facts, image: _image, build: _build),
      InstallStage.naming => InstallNaming(facts),
      InstallStage.wifi => InstallWifi(facts),
      InstallStage.reset => InstallReset(facts),
      InstallStage.handover => InstallHandover(
        facts,
        handover: _handover,
        joinFailed: _joinFailed,
        restartAgain: _restartAgain,
        waitReason: _waitReason,
      ),
      InstallStage.confirm => InstallConfirm(facts, device: _device, deviceAddress: _deviceAddress),
      InstallStage.which => InstallWhich(facts),
      InstallStage.sending => InstallSending(
        facts,
        holdRefused: _holdRefused,
        percent: _percent,
        restarting: _restarting,
      ),
      InstallStage.claim => InstallClaim(
        facts,
        code: List.unmodifiable(_code),
        deadline: _deadline,
        pressDispense: _pressDispense,
        searchingBluetooth: _searchingBluetooth,
        settingUpAgain: _settingUpAgain,
        setupHint: _setupHint,
        windowAsked: windowAsked,
      ),
      InstallStage.finishing => InstallFinishing(
        facts,
        joining: _joining,
        copying: _copying,
        copied: _copied,
        checked: _checked,
        promoting: _promoting,
        written: _written,
        confirming: _confirming,
      ),
      InstallStage.done => InstallDone(facts, backupPath: _backupPath),
      InstallStage.stopped => switch (_stop) {
        final InstallEnd_Stopped stop => InstallStopped(
          facts,
          stop: stop,
          listening: _listening,
          listened: _listened,
          listeningForEarlier: _listeningForEarlier,
          stockSeen: _stockSeen,
          setupSeen: _setupSeen,
          listenerDetail: _listenerDetail,
          promotionMayHaveGone: _promotionMayHaveGone,
        ),
        // Every way into the stage sets the stop first; should one ever not,
        // the first screen stands in rather than a page with no way out.
        null => InstallBefore(facts, image: _image, build: _build),
      },
    };
  }

  @override
  void dispose() {
    _disposed = true;
    _run?.stop();
    if (_onFeederNetwork) {
      _onFeederNetwork = false;
      unawaited(_ports.leaveFeederNetwork());
    }
    unawaited(_updates?.cancel());
    _stopListening();
    _closeLink();
    _holdHintTimer?.cancel();
    unawaited(_power?.cancel());
    unawaited(_release());
    super.dispose();
  }
}
