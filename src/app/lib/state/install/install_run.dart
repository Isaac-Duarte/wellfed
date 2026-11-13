part of 'install.dart';

/// One run: what it reports, what it asks, and how it ends.
extension InstallRunning on InstallFlow {
  Future<void> _start(InstallStart start, String ip, {String? address}) async {
    final InstallPaths? paths = _paths;
    // One run at a time: a second would orphan the first, with no way left
    // to stop it and the broker's port still held.
    if (paths == null || _run != null) return;
    final bool forget = _forget;
    _forget = false;
    final UnjoinedFeeder unjoined = _settingUpAgain
        ? UnjoinedFeeder.setUpAgain
        : _keepWaiting
        ? UnjoinedFeeder.waitForPick
        : UnjoinedFeeder.wait;
    _keepWaiting = false;
    final InstallRun run = _ports.newRun(
      InstallSpec(
        start: start,
        address: address,
        phoneIp: ip,
        home: _home,
        label: _book.label,
        feederName: InstallFlow.defaultFeederName,
        knownFeeders: _book.feeders,
        storePath: paths.store,
        imageDir: paths.image,
        dataDir: paths.data,
        forgetEnrollment: forget,
        feeder: _pickedFeeder,
        unjoined: unjoined,
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
        // Nothing can be found with Bluetooth off. The run asks again every few
        // seconds, so the screen only has to say to turn it on.
        final BluetoothPower power = await bluetoothNow(_ports.bluetooth());
        // The run ended, or the screen went, while the plugin answered: the
        // ask has nobody to answer, and nothing is left to watch for.
        if (_disposed || !identical(_run, run)) return;
        if (power == BluetoothPower.off) {
          _watchBluetooth();
          answer(const InstallAnswer.failed(reason: "this phone's Bluetooth is off"));
          return;
        }
        final InstallLink link;
        try {
          link = await _ports.openLink(
            onBytes: (bytes) => run.notified(bytes: bytes),
            onLost: () {
              run.linkLost();
              unawaited(_lostWithBluetooth(run));
            },
          );
        } on Object catch (error) {
          answer(InstallAnswer.failed(reason: '$error'));
          return;
        }
        // The run ended, or the screen went, while the scan and the connect
        // were out: nothing will ask to close this one, and a feeder still
        // linked does not advertise for the next run's scan.
        if (_disposed || !identical(_run, run)) {
          _closeLink(link);
          return;
        }
        _closeLink(_link);
        _link = link;
        answer(const InstallAnswer.done());
      case InstallAsk_WriteLink(:final pieces):
        final InstallLink? link = _link;
        if (link == null) {
          answer(const InstallAnswer.failed(reason: 'no Bluetooth link is open'));
          return;
        }
        try {
          await link.write(pieces);
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
        await _hold('This $deviceName is serving the firmware.');
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
        _deadline = clock.now().add(Duration(seconds: seconds));
        _windowAsk = null;
      case InstallEvent_PressDispense():
        _pressDispense = true;
      case InstallEvent_KeyKnown():
        _stage = InstallStage.finishing;
      case InstallEvent_Paired(:final identity, :final address, :final rebooting):
        _stage = InstallStage.finishing;
        _identityBytes = identity;
        _identity = _ports.hexOf(identity);
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
      KnownFeeder(identity: identity, name: InstallFlow.defaultFeederName, address: _address),
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
        if (stopped.case_ case StopCase_UncertainWrite(write: UncertainWrite.promotion)) _promotionMayHaveGone = true;
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

  /// A link `run` lost because this phone's Bluetooth went off says so, as a
  /// search with it off does: the owner turns it on, not moves closer.
  Future<void> _lostWithBluetooth(InstallRun run) async {
    final bool off = await bluetoothOffSoon(_ports.bluetooth());
    if (!off || _disposed || !identical(_run, run)) return;
    _watchBluetooth();
    _notify();
  }

  /// Say Bluetooth is off until it comes on.
  void _watchBluetooth() {
    _bluetoothOff = true;
    if (_power != null) return;
    _power = _ports.bluetooth().listen(
      (power) {
        _bluetoothOff = power == BluetoothPower.off;
        if (!_bluetoothOff) {
          unawaited(_power?.cancel());
          _power = null;
        }
        _notify();
      },
      // A plugin that can no longer say can't say it is still off: the search
      // that follows says what is wrong.
      onError: (Object _) {
        _bluetoothOff = false;
        unawaited(_power?.cancel());
        _power = null;
        _notify();
      },
    );
  }

  /// A feeder still linked does not advertise, so a pick-up would never find
  /// it. The run asks for this itself, unless it stopped first.
  /// Hang up `link`, or the open one when none is named.
  void _closeLink([InstallLink? other]) {
    final InstallLink? link = other ?? _link;
    if (identical(link, _link)) _link = null;
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
}
