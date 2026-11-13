import 'dart:async';

import 'package:flutter/foundation.dart';

import 'package:wellfed/platform/restore_ports.dart';
import 'package:wellfed/platform/setup_link.dart' show BluetoothPower;
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/restore.dart';
import 'package:wellfed/state/install/install_state.dart' show CheckState;
import 'package:wellfed/state/restore/restore_state.dart';

export 'package:wellfed/state/restore/restore_state.dart';

enum _Stage { loading, unavailable, noBackup, before, running, done, uncertain, stopped }

/// One restore of a feeder's original firmware.
class RestoreFlow extends ChangeNotifier {
  /// A restore of the feeder `identity`, called [feederName].
  RestoreFlow(this._ports, {required this.identity, required this.feederName, this.address});

  final RestorePorts _ports;

  /// The feeder's identity key.
  final List<int> identity;

  /// What this phone calls the feeder.
  final String feederName;

  /// Where it last answered, if known.
  final String? address;

  /// How the restore stands.
  RestoreState get state => _state;
  RestoreState _state = const RestoreLoading();

  // The working fields of the run, which [_changed] publishes as [state].
  _Stage _stage = _Stage.loading;
  String? _dataDir;
  String? _backup;
  String? _version;
  DateTime? _backupDate;
  bool _picked = false;
  String? _notStock;
  bool _understood = false;
  bool _pressDispense = false;
  CheckState _pressing = CheckState.running;
  CheckState _sending = CheckState.waiting;
  double _written = 0;
  CheckState _restarting = CheckState.waiting;
  bool _sessionDropped = false;
  bool _holdRefused = false;
  RestoreStop? _stop;
  UncertainWhy? _uncertain;
  String? _detail;
  RestoreLook _look = RestoreLook.looking;
  Timer? _lookEnd;
  Timer? _lookAgain;

  /// How many looks have started; a round keeps the count of its own.
  int _looks = 0;
  StreamSubscription<void>? _setup;
  bool _sighted = false;

  /// This phone's Bluetooth is off, so a look can't hear a feeder in setup
  /// mode, and its finding nothing says nothing.
  bool _bluetoothOff = false;
  StreamSubscription<BluetoothPower>? _power;

  /// How long a feeder has to come back: a restart, then its join.
  static const lookFor = Duration(minutes: 3);

  /// Between looks while a feeder may still be restarting.
  static const _lookEvery = Duration(seconds: 5);

  RestoreRun? _run;
  StreamSubscription<RestoreUpdate>? _updates;
  bool _holding = false;
  bool _disposed = false;

  /// Find the copy of stock this phone kept, and look at it. A device that
  /// may not restore is refused first, before anything is read.
  Future<void> load() async {
    if (!_ports.restoresFeeder) {
      _stage = _Stage.unavailable;
      return _changed();
    }
    final String dataDir = await _ports.dataDir();
    _dataDir = dataDir;
    final String? kept = _ports.keptBackup(dataDir, identity);
    if (kept == null) {
      _stage = _Stage.noBackup;
    } else {
      await _lookAt(kept, isPicked: false);
    }
    _changed();
  }

  /// Look at a file the owner picks, through the same checks as a kept copy.
  Future<void> pickFile() async {
    final String? path = await _ports.pickBackup();
    if (path == null) return;
    await _lookAt(path, isPicked: true);
    _changed();
  }

  Future<void> _lookAt(String path, {required bool isPicked}) async {
    switch (_ports.checkBackup(path)) {
      case BackupCheck_Ready(version: final String ready):
        _backup = path;
        _version = ready;
        _backupDate = await _ports.backupDate(path);
        _picked = isPicked;
        _notStock = null;
        _stage = _Stage.before;
      case BackupCheck_Missing():
        _notStock = null;
        _stage = _Stage.noBackup;
      case BackupCheck_NotStock(detail: final String why):
        _notStock = why;
        _stage = _Stage.noBackup;
    }
  }

  /// The owner gave or took back their word.
  void setUnderstood(bool value) {
    _understood = value;
    _changed();
  }

  /// The owner's word is what starts it, each time.
  Future<void> start() async {
    final (String? dataDir, String? backup) = (_dataDir, _backup);
    // `_run` is set only after the hold below, so the stage guards a second tap.
    if (!_understood || _stage != _Stage.before || dataDir == null || backup == null) {
      return;
    }
    _pressDispense = false;
    _pressing = CheckState.running;
    _sending = CheckState.waiting;
    _restarting = CheckState.waiting;
    _written = 0;
    _sessionDropped = false;
    _stop = null;
    _uncertain = null;
    _detail = null;
    _stage = _Stage.running;
    _changed();

    _holding = true;
    _holdRefused = !await _ports.holdProcess('Putting the original firmware back on $feederName');
    if (_disposed) {
      // The screen went away during the wait, and its release ran before this
      // hold took effect.
      await _ports.releaseProcess();
      return;
    }

    final RestoreRun run = _ports.newRun(
      RestoreSpec(identity: Uint8List.fromList(identity), address: address, dataDir: dataDir, backup: backup),
    );
    _run = run;
    _updates = run.run().listen(_heard, onDone: _closed);
    _changed();
  }

  /// Back at the facts after a stop that left the way open. The owner gives
  /// their word again.
  void again() {
    _stopLook();
    _understood = false;
    _stage = _Stage.before;
    _changed();
  }

  void _heard(RestoreUpdate update) {
    switch (update) {
      case RestoreUpdate_Event(:final event):
        _event(event);
      case RestoreUpdate_Ask(:final id, :final ask):
        unawaited(_ask(id, ask));
      case RestoreUpdate_Ended(:final end):
        _ended(end);
    }
    _changed();
  }

  void _event(InstallEvent event) {
    switch (event) {
      case InstallEvent_PressDispense():
        _pressDispense = true;
      case InstallEvent_Promoting():
        _pressDispense = false;
        _pressing = CheckState.done;
        _sending = CheckState.running;
      case InstallEvent_Written(:final written, :final length):
        _written = length == 0 ? 0 : written / length;
        _sessionDropped = false;
      case InstallEvent_SessionDropped():
        _sessionDropped = true;
      case InstallEvent_Activated():
        _sending = CheckState.done;
        _restarting = CheckState.running;
      default:
        break;
    }
  }

  Future<void> _ask(int id, InstallAsk ask) async {
    final InstallAnswer answer;
    if (ask case InstallAsk_Locate(:final identity)) {
      answer = InstallAnswer.located(address: await _ports.locate(identity));
    } else {
      answer = const InstallAnswer.failed(reason: 'A restore asks only where the feeder is');
    }
    _run?.answer(id: id, answer: answer);
  }

  void _ended(RestoreEnd end) {
    switch (end) {
      case RestoreEnd_Activated():
        _stage = _Stage.done;
      case RestoreEnd_Uncertain(:final why, detail: final words):
        _uncertain = why;
        _detail = words;
        _stage = _Stage.uncertain;
        _startLook();
      case RestoreEnd_Stopped(:final case_, detail: final words):
        _stop = case_;
        _detail = words;
        _stage = _Stage.stopped;
    }
    _finish();
  }

  /// The stream closed with no end: a run that says nothing more has not
  /// said how it ended, so after an offer it is uncertain.
  void _closed() {
    if (_stage != _Stage.running) return;
    _stage = _sending == CheckState.waiting ? _Stage.stopped : _Stage.uncertain;
    if (_stage == _Stage.stopped) _stop = RestoreStop.other;
    if (_stage == _Stage.uncertain) {
      _uncertain = UncertainWhy.failed;
      _startLook();
    }
    _finish();
    _changed();
  }

  void _finish() {
    _run = null;
    unawaited(_updates?.cancel());
    _updates = null;
    if (_holding) {
      _holding = false;
      unawaited(_ports.releaseProcess());
    }
    if (_disposed) _forget();
  }

  /// The owner has left: a live key has no expiry, so it does not outlast
  /// them. Only a [start] from the same screen reuses it.
  void _forget() {
    if (_dataDir case final dataDir?) _ports.forgetKey(dataDir, identity);
  }

  /// Look for the feeder, since the words for an uncertain end depend on it,
  /// from the moment the run ends uncertain: a verdict an earlier run's look
  /// left is never shown for this one. Wellfed advertises on the network with
  /// its people in place, or Bluetooth setup with them cleared; stock does
  /// neither. This reads and listens, and offers the feeder nothing.
  void _startLook() {
    _stopLook();
    _looks++;
    _look = RestoreLook.looking;
    _sighted = false;
    _lookEnd = Timer(lookFor, () => _looked(_sighted ? RestoreLook.sighted : RestoreLook.nothing));
    _setup = _ports.setupHeard().listen((_) => _looked(RestoreLook.setup), onError: (Object _) {});
    unawaited(_locate(_looks));
    _watchBluetooth();
  }

  /// Ask Android to turn this phone's Bluetooth on.
  Future<void> turnOnBluetooth() => _ports.turnOnBluetooth();

  /// Follow this phone's Bluetooth while a look may need it. Off at any point,
  /// the look could miss a feeder in setup mode, so its finding nothing says
  /// nothing: the screen says Bluetooth is off. Back on, a look without a
  /// verdict it could trust starts again from the start.
  void _watchBluetooth() {
    if (_power != null) return;
    _power = _ports.bluetooth().listen((power) {
      final bool wasOff = _bluetoothOff;
      _bluetoothOff = power == BluetoothPower.off;
      if (wasOff && !_bluetoothOff && _stage == _Stage.uncertain) {
        if (_look case RestoreLook.looking || RestoreLook.nothing || RestoreLook.sighted) _startLook();
      }
      _changed();
    }, onError: (Object _) {});
  }

  /// One round of look `look`, which a later look's start leaves to lapse:
  /// a search or read still out from an earlier run decides nothing for
  /// this one, and starts no second chain of rounds.
  Future<void> _locate(int look) async {
    bool current() => look == _looks && _look == RestoreLook.looking && _lookEnd != null;

    final String? found = await _ports.locate(_ports.hexOf(identity));
    if (!current()) return;
    if (found != null) {
      _sighted = true;
      final RestoreSeen seen = await _ports.readFeeder(identity, found);
      if (!current()) return;
      if (seen == RestoreSeen.idle) {
        _looked(RestoreLook.wellfed);
        return;
      }
    }
    _lookAgain = Timer(_lookEvery, () => unawaited(_locate(look)));
  }

  void _looked(RestoreLook found) {
    _stopLook();
    _look = found;
    _changed();
  }

  void _stopLook() {
    _lookEnd?.cancel();
    _lookEnd = null;
    _lookAgain?.cancel();
    _lookAgain = null;
    unawaited(_setup?.cancel());
    _setup = null;
  }

  void _changed() {
    _state = _snapshot();
    if (!_disposed) notifyListeners();
  }

  RestoreState _snapshot() => switch (_stage) {
    _Stage.loading => const RestoreLoading(),
    _Stage.unavailable => const RestoreUnavailable(),
    _Stage.noBackup => RestoreNoBackup(notStock: _notStock),
    _Stage.before => RestoreBefore(
      version: _version,
      backupDate: _backupDate,
      picked: _picked,
      understood: _understood,
    ),
    _Stage.running => RestoreRunning(
      pressDispense: _pressDispense,
      pressing: _pressing,
      sending: _sending,
      written: _written,
      restarting: _restarting,
      sessionDropped: _sessionDropped,
      holdRefused: _holdRefused,
    ),
    _Stage.done => RestoreDone(version: _version),
    _Stage.uncertain => RestoreUncertain(
      why: _uncertain ?? UncertainWhy.failed,
      detail: _detail,
      look: _look,
      bluetoothOff: _bluetoothOff,
    ),
    _Stage.stopped => RestoreStopped(stop: _stop ?? RestoreStop.other, detail: _detail),
  };

  @override
  void dispose() {
    _disposed = true;
    _stopLook();
    unawaited(_power?.cancel());
    if (_run case final run?) {
      // Its end does the rest: the key goes once the run can no longer use it.
      run.stop();
    } else {
      _finish();
    }
    super.dispose();
  }
}
