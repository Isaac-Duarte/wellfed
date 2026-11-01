import 'dart:async';

import 'package:flutter/foundation.dart';

import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/setup.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/install/install_state.dart';

export 'package:wellfed/state/install/install_state.dart';

/// One install, from the first screen to its end.
class InstallFlow extends ChangeNotifier {
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

  /// Android refused Bluetooth to a pick-up that may need it.
  bool _bluetoothRefused = false;
  bool _permitted = false;

  InstallRun? _run;
  StreamSubscription<InstallUpdate>? _updates;
  InstallLink? _link;
  InstallListener? _listener;
  StreamSubscription<ListenUpdate>? _listenUpdates;
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

  /// Read what decides whether this phone can install at all.
  Future<void> load() async {
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

  /// The owner held Reset: start the broker, then the handover. A feeder that
  /// kept an earlier run's frame only has to reach this phone again.
  Future<void> startHandover() async {
    if (_run != null) return;
    _phoneIp = await _ports.phoneAddress();
    final String? ip = _phoneIp;
    if (ip == null) {
      _notify();
      return;
    }
    // Stock kept the address the frame named, so it looks nowhere else; a new
    // frame is the only way to tell it this one, and that takes another hold.
    if (_frameTaken && ip != _frameAddress) {
      _frameTaken = false;
      _feederReset = false;
      _go(InstallStage.reset);
      return;
    }
    _feederReset = true;
    _joinFailed = false;
    _handover = _frameTaken ? Handover.waiting : Handover.starting;
    _go(InstallStage.handover);
    await _start(_frameTaken ? InstallStart.frameTaken : InstallStart.reset, ip);
  }

  /// Join the feeder's network. The owner may try again: the picker comes up
  /// empty until the feeder's network is up.
  Future<void> joinFeeder() async {
    final int? ask = _joinAsk;
    final InstallRun? run = _run;
    if (ask == null || run == null || _handover == Handover.joining) return;
    _handover = Handover.joining;
    _joinFailed = false;
    _notify();
    try {
      await _ports.joinFeederNetwork();
    } on Object catch (_) {
      _handover = Handover.join;
      _joinFailed = true;
      _notify();
      return;
    }
    // The run ended while the picker was up: nothing will ever ask to leave.
    if (_disposed || !identical(_run, run)) {
      await _ports.leaveFeederNetwork();
      return;
    }
    _joinAsk = null;
    _onFeederNetwork = true;
    _handover = Handover.sending;
    _notify();
    run.answer(id: ask, answer: const InstallAnswer.done());
  }

  /// Back on the Wi-Fi: the frame named this phone's address, so another one
  /// would leave the feeder looking in the wrong place.
  Future<void> _leaveFeeder(void Function(InstallAnswer) answer) async {
    final String? address = await _ports.leaveFeederNetwork();
    _onFeederNetwork = false;
    if (address == _phoneIp && address != null) {
      _handover = Handover.waiting;
      answer(const InstallAnswer.done());
      return;
    }
    answer(
      InstallAnswer.failed(
        reason:
            'This phone came back to its Wi-Fi at ${address ?? "no address"}, '
            'but told the feeder $_phoneIp.',
      ),
    );
  }

  /// A feeder already on the replacement, with no record of it: nothing says
  /// which one unless this phone knows none.
  void alreadyRuns() {
    if (_book.feeders.isEmpty) {
      unawaited(resume());
    } else {
      _go(InstallStage.which);
    }
  }

  /// The owner's answer: a known feeder by its identity, or `null` for another,
  /// over Bluetooth.
  Future<void> pickFeeder(String? identity) async {
    _pickedFeeder = identity;
    await resume();
  }

  /// Pick up from bank 0: the replacement runs, and only its owner is
  /// missing, or the backup and the promotion. With `address`, it answers
  /// there. A consumer image that never paired needs its Wi-Fi for Bluetooth
  /// setup, which only an earlier run on this screen would have asked for.
  Future<void> resume({String? address}) async {
    if (_run != null) return;
    if (!await _permissions()) {
      _notify();
      return;
    }
    _stopListening();
    _listening = Listening.idle;
    if (_book.label.isEmpty) {
      _resumeAfterWifi = true;
      _resumeAddress = address;
      _stop = null;
      _go(InstallStage.naming);
      return;
    }
    if (provisioning == ImageProvisioning.consumer &&
        _pending != PendingInstall.paired &&
        _pickedFeeder == null &&
        address == null &&
        _home == null) {
      _resumeAfterWifi = true;
      _resumeAddress = address;
      _stop = null;
      _go(InstallStage.wifi);
      return;
    }
    final String ip = _phoneIp ?? await _ports.phoneAddress() ?? '';
    _stop = null;
    _resetProgress();
    _go(InstallStage.claim);
    await _start(InstallStart.bank0, ip, address: address);
  }

  /// A request from an earlier run has no settled outcome: listen for what
  /// became of it, as a run that just stopped there would.
  Future<void> listenForOpenRequest() async {
    if (!await _permissions()) {
      _notify();
      return;
    }
    _stop = InstallEnd_Stopped(case_: const StopCase.uncertain(), resume: Resume.nothing, detail: _stop?.detail ?? '');
    _listeningForEarlier = true;
    _go(InstallStage.stopped);
    await listen();
  }

  void _resetProgress() {
    _code = const [];
    _deadline = null;
    _windowAsk = null;
    _pressDispense = false;
    _percent = 0;
    _restarting = false;
    _joining = CheckState.waiting;
    _copying = CheckState.waiting;
    _copied = 0;
    _checked = CheckState.waiting;
    _promoting = CheckState.waiting;
    _written = 0;
    _confirming = CheckState.waiting;
    _searchingBluetooth = false;
    _foundOverBluetooth();
  }

  /// A feeder that never joined: take a new Wi-Fi and go through Bluetooth
  /// setup again. Only stock's request cannot be sent twice, and this asks stock
  /// nothing.
  void newWifiForSetup() {
    _settingUpAgain = true;
    _pickedFeeder = null;
    _home = null;
    _resumeAfterWifi = true;
    _resumeAddress = null;
    _stop = null;
    _go(InstallStage.wifi);
  }

  /// Wi-Fi the feeder could not be given, or none: ask again, and go on by the
  /// route that stopped.
  void changeWifi() {
    _home = null;
    _resumeAfterWifi = _stop?.resume == Resume.fromBank0;
    _resumeAddress = null;
    _stop = null;
    _go(InstallStage.wifi);
  }

  /// A feeder that never joined may still be restarting: look again.
  Future<void> keepWaiting() async {
    _settingUpAgain = false;
    await again();
  }

  /// The stop's way on: from the start, or from bank 0, as the run said.
  Future<void> again({bool forget = false}) async {
    _forget = forget;
    final Resume? resumeFrom = _stop?.resume;
    if (resumeFrom == Resume.fromBank0) {
      // No enrollment leaves no record, but this flow knows the feeder it
      // paired. Setup would send the owner to a factory reset, and lose the
      // identity its backup belongs to. Only a new Wi-Fi needs setup again, and
      // the other ways into resume are the owner asking for Bluetooth.
      if (_pending != PendingInstall.paired && !_settingUpAgain) {
        _pickedFeeder ??= _identity;
      }
      await resume();
      return;
    }
    if (!await _permissions()) {
      _notify();
      return;
    }
    _phoneIp = await _ports.phoneAddress();
    _stop = null;
    _resetProgress();
    _listening = Listening.idle;
    _listeningForEarlier = false;
    _waitReason = null;
    _restartAgain = false;
    // A fresh launch can get here from an open request, with neither asked yet.
    if (_book.label.isEmpty) {
      _go(InstallStage.naming);
    } else if (_home == null) {
      _go(InstallStage.wifi);
    } else if (_feederReset) {
      await startHandover();
    } else {
      _go(InstallStage.reset);
    }
  }

  Future<void> _start(InstallStart start, String ip, {String? address}) async {
    final InstallPaths? paths = _paths;
    // One run at a time: a second would orphan the first, with no way left
    // to stop it and the broker's port still held.
    if (paths == null || _run != null) return;
    final bool forget = _forget;
    _forget = false;
    final InstallRun run = _ports.newRun(
      InstallSpec(
        start: start,
        address: address,
        phoneIp: ip,
        home: _home,
        label: _book.label,
        feederName: defaultFeederName,
        knownFeeders: _book.feeders,
        storePath: paths.store,
        imageDir: paths.image,
        dataDir: paths.data,
        forgetEnrollment: forget,
        feeder: _pickedFeeder,
        setupAgain: _settingUpAgain,
      ),
    );
    _run = run;
    _notify();
    await _hold('Keep the feeder plugged in.');
    if (_disposed) return;
    InstallEnd lost(String detail) =>
        InstallEnd.stopped(case_: const StopCase.other(), resume: Resume.nothing, detail: detail);
    _updates = run.run().listen(
      (update) => unawaited(_update(run, update)),
      onError: (Object error) => _ended(lost('$error')),
      // A run's last update is its end, which cancels this first; a stream
      // that closes without one lost its thread.
      onDone: () {
        if (identical(_run, run)) _ended(lost('the install stopped answering'));
      },
    );
    _notify();
  }

  Future<void> _hold(String text) async {
    _holding = true;
    _holdRefused = !await _ports.holdProcess(text);
    // The screen went away during the wait, and its release ran before this
    // hold took effect.
    if (_disposed) await _ports.releaseProcess();
  }

  Future<void> _release() async {
    if (!_holding) return;
    _holding = false;
    await _ports.releaseProcess();
  }

  Future<void> _update(InstallRun run, InstallUpdate update) async {
    if (_disposed) return;
    switch (update) {
      case InstallUpdate_Event(:final event):
        await _event(event);
      case InstallUpdate_Ask(:final id, :final ask):
        await _ask(run, id, ask);
      case InstallUpdate_Ended(:final end):
        _ended(end);
    }
    _notify();
  }

  Future<void> _ask(InstallRun run, int id, InstallAsk ask) async {
    void answer(InstallAnswer answer) => run.answer(id: id, answer: answer);

    switch (ask) {
      // The screen says to restart it; the broker is already listening.
      case InstallAsk_RestartFeeder():
        answer(const InstallAnswer.done());
      case InstallAsk_JoinFeederNetwork():
        _joinAsk = id;
        _handover = Handover.join;
      case InstallAsk_LeaveFeederNetwork():
        await _leaveFeeder(answer);
      case InstallAsk_RecognizeFeeder(device: final heard, :final address):
        // Whatever reached the broker took the frame, and has no setup network
        // to take another.
        _frameTaken = true;
        _frameAddress = _phoneIp;
        _device = heard;
        _deviceAddress = address;
        _recognizeAsk = id;
        _stage = InstallStage.confirm;
      case InstallAsk_OpenRecoveryWindow():
        _windowAsk = id;
        _stage = InstallStage.claim;
      case InstallAsk_OpenLink():
        _stage = InstallStage.claim;
        _searchBluetooth();
        try {
          _link = await _ports.openLink(
            onBytes: (bytes) => run.notified(bytes: bytes),
            onLost: run.linkLost,
          );
          answer(const InstallAnswer.done());
        } on Object catch (error) {
          answer(InstallAnswer.failed(reason: '$error'));
        }
      case InstallAsk_WriteLink(:final pieces):
        try {
          await _link?.write(pieces);
          answer(const InstallAnswer.done());
        } on Object catch (error) {
          answer(InstallAnswer.failed(reason: '$error'));
        }
      case InstallAsk_CloseLink():
        try {
          await _link?.close();
        } on Object catch (_) {
          // A link the feeder dropped has nothing left to close.
        }
        _link = null;
        answer(const InstallAnswer.done());
      case InstallAsk_Locate(:final identity):
        final String? address = await _ports.locate(identity);
        answer(InstallAnswer.located(address: address));
    }
  }

  Future<void> _event(InstallEvent event) async {
    switch (event) {
      case InstallEvent_Waiting(:final why):
        _waitReason = why;
      case InstallEvent_VersionLost():
        _restartAgain = true;
      case InstallEvent_Requesting():
        _stage = InstallStage.sending;
        _sendingSince = DateTime.now();
        await _hold('This phone is serving the firmware.');
      case InstallEvent_StockProgress(percent: final sent):
        _stage = InstallStage.sending;
        _percent = sent;
      case InstallEvent_StockRestarting():
        _stage = InstallStage.sending;
        _restarting = true;
      case InstallEvent_SetupStarting():
        _stage = InstallStage.claim;
        _searchBluetooth();
        await _hold('The feeder is waiting for its owner.');
      case InstallEvent_Code(code: final shown, :final seconds):
        _stage = InstallStage.claim;
        _code = shown;
        _foundOverBluetooth();
        _deadline = DateTime.now().add(Duration(seconds: seconds));
        _windowAsk = null;
      case InstallEvent_PressDispense():
        _pressDispense = true;
      case InstallEvent_KeyKnown():
        _stage = InstallStage.finishing;
      case InstallEvent_Paired(:final identity, :final address, :final rebooting):
        _stage = InstallStage.finishing;
        _identityBytes = identity;
        _identity = identity.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join();
        _address = address ?? _address;
        _joining = rebooting ? CheckState.running : CheckState.waiting;
      case InstallEvent_Joined(:final address):
        _stage = InstallStage.finishing;
        _settingUpAgain = false;
        _address = address;
        _joining = CheckState.done;
      case InstallEvent_ReplacementAnswers():
        _stage = InstallStage.finishing;
        _pressDispense = false;
      case InstallEvent_ReadingBackup():
        _copying = CheckState.running;
        await _hold('Copying the original firmware.');
      case InstallEvent_BackupRead(:final bytes, :final total):
        _copying = CheckState.running;
        _copied = total == 0 ? 0 : bytes.toDouble() / total;
      case InstallEvent_BackupKept(:final path):
        _backupPath = path;
        _copying = CheckState.done;
        _checked = CheckState.done;
      case InstallEvent_Promoting():
        _copying = CheckState.done;
        _checked = CheckState.done;
        _promoting = CheckState.running;
        await _hold('Making Wellfed permanent.');
      case InstallEvent_Written(written: final done, :final length):
        _written = length == 0 ? 0 : done / length;
      case InstallEvent_Activated():
        _copying = CheckState.done;
        _checked = CheckState.done;
        _promoting = CheckState.done;
        _confirming = CheckState.running;
      case InstallEvent_Installed():
        _confirming = CheckState.done;
      case InstallEvent_SessionDropped():
      case InstallEvent_StockServerStarted() ||
          InstallEvent_AwaitingStock() ||
          InstallEvent_StockServerEnded() ||
          InstallEvent_NoSetupFeeder():
        break;
      case InstallEvent_Note(:final text):
        debugPrint('install: $text');
    }
  }

  /// The feeder enrolled this phone, and Rust saved it in the app's file
  /// before going on. Home hears of it only here, as the run ends: a session
  /// of its own would only get in the install's way.
  Future<void> _settle() async {
    final String? identity = _identity;
    if (identity == null) return;
    await _book.remember(
      KnownFeeder(identity: identity, name: defaultFeederName, address: _address),
      paired: true,
      // A pick-up of a feeder the owner already named.
      keepName: true,
    );
  }

  void _ended(InstallEnd end) {
    _run = null;
    _joinAsk = null;
    if (_onFeederNetwork) {
      _onFeederNetwork = false;
      unawaited(_ports.leaveFeederNetwork());
    }
    unawaited(_updates?.cancel());
    _updates = null;
    unawaited(_settle());
    _closeLink();
    _foundOverBluetooth();
    if (end case InstallEnd_Stopped(case_: StopCase_NoFeederCame() || StopCase_TwoFeeders())) {
      _frameTaken = false;
    }
    switch (end) {
      case InstallEnd_Done():
        _stage = InstallStage.done;
        // A pick-up past the backup never read it again.
        final (List<int>? identity, InstallPaths? paths) = (_identityBytes, _paths);
        if (_backupPath == null && identity != null && paths != null) {
          _backupPath = _ports.keptBackup(paths, identity);
        }
        unawaited(_release());
      case final InstallEnd_Stopped stopped:
        _stop = stopped;
        _readPending();
        _stage = InstallStage.stopped;
        _windowAsk = null;
        _recognizeAsk = null;
        if (stopped.case_ is StopCase_Uncertain) {
          unawaited(listen());
        } else {
          unawaited(_release());
        }
    }
    _notify();
  }

  /// A feeder still linked does not advertise, so a pick-up would never find
  /// it. The run asks for this itself, unless it stopped first.
  void _closeLink() {
    final InstallLink? link = _link;
    _link = null;
    unawaited(
      link?.close().catchError((Object _) {
        // A link the feeder dropped has nothing left to close.
      }),
    );
  }

  /// The owner's word on the feeder that connected.
  void recognize({required bool yes}) {
    final int? id = _recognizeAsk;
    if (id == null) return;
    _recognizeAsk = null;
    // Somebody else's client: the feeder may still be waiting for its frame.
    if (!yes) _frameTaken = false;
    _run?.answer(
      id: id,
      answer: InstallAnswer.yes(yes: yes),
    );
    if (yes) {
      _stage = InstallStage.sending;
      _sendingSince = DateTime.now();
    }
    _notify();
  }

  /// A bench image's recovery window is open.
  void windowOpened() {
    final int? id = _windowAsk;
    if (id == null) return;
    _windowAsk = null;
    _run?.answer(id: id, answer: const InstallAnswer.done());
    _notify();
  }

  /// Listen for what became of an uncertain request. Sends the feeder
  /// nothing: only what it does next can say.
  Future<void> listen() async {
    final InstallPaths? paths = _paths;
    if (paths == null) return;
    _stopListening();
    _listening = Listening.running;
    _listened = Duration.zero;
    _stockSeen = false;
    _setupSeen = false;
    _notify();
    await _hold('Listening for the feeder. Leave it plugged in.');
    if (_disposed || _listening != Listening.running) return;
    final InstallListener listener = _ports.newListener(paths);
    _listener = listener;
    _listenUpdates = listener.run().listen(
      _heardFromListener,
      onError: (Object error) {
        _listenerDetail = '$error';
        _listenEnded(Listening.failed);
      },
      // Its last update ends the listening; a stream that closes before one
      // lost its thread.
      onDone: () {
        if (identical(_listener, listener) && _listening == Listening.running) {
          _listenerDetail = 'the listener stopped answering';
          _listenEnded(Listening.failed);
        }
      },
    );
    // A consumer image has no network until setup gives it one, so it is
    // only heard advertising setup. Any feeder in setup mode sounds the same,
    // so this keeps listening until the owner says it is this one.
    _heard = _ports.setupHeard().listen((_) {
      _setupSeen = true;
      _notify();
    }, onError: (Object _) {});
    _notify();
  }

  void _heardFromListener(ListenUpdate update) {
    switch (update) {
      case ListenUpdate_Tick(:final seconds, :final stock):
        _listened = Duration(seconds: seconds);
        _stockSeen = stock;
        _notify();
      case ListenUpdate_Replacement(:final address):
        unawaited(resume(address: address));
      case ListenUpdate_StockStayed(:final resolved):
        _stop = InstallEnd_Stopped(
          case_: const StopCase.uncertain(),
          resume: resolved ? Resume.fromStart : Resume.nothing,
          detail: _stop?.detail ?? '',
        );
        _listenEnded(Listening.stockStayed);
      case ListenUpdate_Silent():
        _listenEnded(Listening.silent);
      case ListenUpdate_Failed(:final detail):
        _listenerDetail = detail;
        _listenEnded(Listening.failed);
      case ListenUpdate_Stopped():
        break;
    }
  }

  void _listenEnded(Listening end) {
    _listening = end;
    unawaited(_heard?.cancel());
    _heard = null;
    _readPending();
    unawaited(_release());
    _notify();
  }

  void _stopListening() {
    _listener?.stop();
    _listener = null;
    unawaited(_listenUpdates?.cancel());
    _listenUpdates = null;
    unawaited(_heard?.cancel());
    _heard = null;
  }

  void _readPending() {
    final InstallPaths? paths = _paths;
    if (paths != null) _pending = _ports.pending(paths);
  }

  /// The way out of a stuck or damaged record is open: nothing is using it.
  bool get mayForget => _paths != null && _run == null && _listening != Listening.running;

  /// A stop that leaves a record behind, or one no pick-up gets past. Not offered
  /// on an uncertain write's stop: the feeder may be mid-way, and a pick-up reads
  /// where it got to. A reopened screen has no such stop to go by and does offer
  /// it, which is recoverable: the first screen's action reads the feeder and
  /// sends nothing. A damaged ledger has an action of its own, which this cannot do.
  bool get forgetOffered {
    if (!mayForget) return false;
    final StopCase? case_ = _stop?.case_;
    if (case_ is StopCase_UncertainWrite) return false;
    if (_pending == PendingInstall.ledgerDamaged) return false;
    return _pending != PendingInstall.nothing || case_ is StopCase_OtherFeeder || case_ is StopCase_DamagedRecord;
  }

  /// Forgetting takes away a working "Pick up where it stopped", so its words
  /// say how to come back.
  bool get forgetEndsPickUp => _pending == PendingInstall.paired || _stop?.resume == Resume.fromBank0;

  /// Some feeder's install request has no known outcome, so forgetting needs
  /// words of its own.
  bool get requestStillOpen {
    final InstallPaths? paths = _paths;
    return paths != null && _ports.requestOpen(paths);
  }

  /// Why the last repair of a record failed, when it did.
  String? _repairFailed;

  /// Whether `repair` ran; a failure is kept for the screen.
  bool _repair(void Function(InstallPaths) repair) {
    final InstallPaths? paths = _paths;
    if (paths == null || !mayForget) return false;
    try {
      repair(paths);
    } on BridgeError catch (failed) {
      _repairFailed = failed.message;
      _notify();
      return false;
    }
    _repairFailed = null;
    return true;
  }

  /// Back to the first screen, knowing nothing of any run.
  void _startClean() {
    _stop = null;
    _frameTaken = false;
    _frameAddress = null;
    _feederReset = false;
    _identity = null;
    _identityBytes = null;
    _address = null;
    _backupPath = null;
    _settingUpAgain = false;
    _waitReason = null;
    _restartAgain = false;
    _pickedFeeder = null;
    _listening = Listening.idle;
    _listeningForEarlier = false;
    _resetProgress();
    _readPending();
    _go(InstallStage.before);
  }

  /// Clear the install in progress and go back to the first screen. The backups
  /// stay, and the ledger keeps a request with no known outcome from going to
  /// that feeder a second time.
  void forgetInstall() {
    if (_repair(_ports.forgetInstall)) _startClean();
  }

  /// The owner accepted that this phone no longer knows whether a request to the
  /// feeder a damaged ledger was about is open.
  void setAsideLedgers() {
    if (_repair(_ports.setAsideLedgers)) _startClean();
  }

  /// The owner accepted that the backup at `path` is no longer one to go back
  /// with. A pick-up then reads a fresh one if the feeder still holds stock.
  Future<void> setAsideBackup(String path, BackupProblem problem) async {
    final bool done = _repair((paths) => _ports.setAsideBackup(paths, path, problem));
    if (done) await again();
  }

  /// End the run through its cleanup; the end says what it left.
  void stopRun() => _run?.stop();

  /// The feeder's name on this phone, and where it last answered.
  Future<void> finish(String name) async {
    final String? identity = _identity;
    if (identity == null) return;
    await _book.remember(KnownFeeder(identity: identity, name: name, address: _address));
  }

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
      repairFailed: _repairFailed,
      phoneIp: _phoneIp,
    );
    return switch (_stage) {
      InstallStage.loading => InstallLoading(facts),
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
      InstallStage.stopped => InstallStopped(
        facts,
        stop: _stop,
        listening: _listening,
        listened: _listened,
        listeningForEarlier: _listeningForEarlier,
        stockSeen: _stockSeen,
        setupSeen: _setupSeen,
        listenerDetail: _listenerDetail,
      ),
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
    unawaited(_release());
    super.dispose();
  }
}
