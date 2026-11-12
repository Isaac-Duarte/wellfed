part of 'install.dart';

/// Listening after an uncertain request, and repairing what a run left.
extension InstallListening on InstallFlow {
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
      case ListenUpdate_Failed(:final why, :final detail):
        _listenerDetail = detail;
        _listenEnded(switch (why) {
          ListenFailure.ledgerDamaged => Listening.ledgerDamaged,
          ListenFailure.serverFailed => Listening.serverFailed,
          ListenFailure.other => Listening.failed,
        });
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
    _promotionMayHaveGone = false;
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
}
