part of 'install.dart';

/// The owner's way through an install, up to where a run starts.
extension InstallSteps on InstallFlow {
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
            'This $deviceName came back to its Wi-Fi at ${address ?? "no address"}, '
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
    _keepWaiting = false;
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
    _keepWaiting = true;
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
}
