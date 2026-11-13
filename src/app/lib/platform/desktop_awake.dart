import 'dart:async';
import 'dart:ffi';
import 'dart:io';

/// Starts a helper process, as [Process.start] does.
typedef StartHelper = Future<Process> Function(String executable, List<String> arguments);

/// Keeps a computer from sleeping while a run holds it: an install, an
/// update or a restore. A computer that sleeps mid-run stops serving the
/// feeder (the broker and the image server of an install) or drops the
/// session pushing its firmware, so the run would stop partway.
///
/// Each desktop has its own way, and none needs a dependency:
///
/// - macOS: `caffeinate -i -w <this process>`, a power assertion against idle
///   sleep that ends when this process does, even if it crashes.
/// - Linux: `systemd-inhibit --what=idle:sleep`, holding logind's inhibitor
///   for as long as its child, `cat` on a pipe from this process, runs. The
///   pipe closes when this process ends, so a crash cannot leave it held.
/// - Windows: `SetThreadExecutionState` on the platform thread, which the
///   main isolate runs on and which lives as long as the app.
///
/// None of them stops sleep from closing a laptop's lid, so the screens say
/// to keep it open.
class DesktopAwake {
  /// Helpers started through `start`, for a test to stand in. `onLapsed`
  /// hears of a helper that ended while a run still held.
  DesktopAwake({StartHelper? start, String? os, this._onLapsed})
    : _start = start ?? Process.start,
      _os = os ?? Platform.operatingSystem;

  final StartHelper _start;
  final String _os;
  final void Function()? _onLapsed;

  /// How long a helper that started may take to refuse. systemd-inhibit asks
  /// logind for the lock before it starts its child, and exits at once when
  /// logind refuses or there is no session; caffeinate exits at once on a bad
  /// process id.
  static const refusalWait = Duration(milliseconds: 250);

  /// The helper holding the computer awake on macOS and Linux.
  Process? _helper;

  /// Whether Windows holds it now.
  bool _threadHeld = false;

  /// Counts releases, so a hold whose helper is still being started learns a
  /// release came meanwhile.
  int _released = 0;

  /// Hold the computer awake while `text` names a run, or let it sleep again
  /// once it is `null`. Whether it is held.
  Future<bool> apply(String? text) async {
    if (text == null) {
      await _release();
      return false;
    }
    return _hold(text);
  }

  Future<bool> _hold(String why) async {
    if (_helper != null || _threadHeld) return true;
    // A second run's hold while the first's helper starts shares it.
    if (_starting case final starting?) return starting;
    final Future<bool> starting = _holdNow(why);
    _starting = starting;
    try {
      return await starting;
    } finally {
      _starting = null;
    }
  }

  /// A helper being started, until it holds or is refused.
  Future<bool>? _starting;

  Future<bool> _holdNow(String why) async {
    switch (_os) {
      case 'windows':
        _threadHeld = _windowsState(_esContinuous | _esSystemRequired) != 0;
        return _threadHeld;
      case 'macos':
        return _startHelper('/usr/bin/caffeinate', ['-i', '-w', '$pid']);
      case 'linux':
        return _startHelper('systemd-inhibit', [
          '--what=idle:sleep',
          '--who=Wellfed',
          '--why=$why',
          '--mode=block',
          'cat',
        ]);
      default:
        return false;
    }
  }

  Future<bool> _startHelper(String executable, List<String> arguments) async {
    final int released = _released;
    final Process helper;
    try {
      helper = await _start(executable, arguments);
    } on ProcessException {
      // No helper on this computer, as on a Linux without systemd: the run
      // goes on, and the screen says to keep the computer awake.
      return false;
    }
    // Neither helper writes anything, but a pipe nobody reads must not fill.
    unawaited(helper.stdout.drain<void>());
    unawaited(helper.stderr.drain<void>());
    final bool refused = await Future.any([
      helper.exitCode.then((_) => true),
      Future<bool>.delayed(refusalWait, () => false),
    ]);
    if (refused) return false;
    // Released while it started: nothing holds any more, so neither may it.
    if (released != _released) {
      await _end(helper);
      return false;
    }
    _helper = helper;
    unawaited(
      helper.exitCode.then((_) {
        // Released helpers are no longer `_helper`: only one that ended by
        // itself, with a run holding, gets here.
        if (!identical(_helper, helper)) return;
        _helper = null;
        _onLapsed?.call();
      }),
    );
    return true;
  }

  Future<void> _release() async {
    _released++;
    if (_threadHeld) {
      _threadHeld = false;
      _windowsState(_esContinuous);
    }
    final Process? helper = _helper;
    _helper = null;
    if (helper != null) await _end(helper);
  }

  Future<void> _end(Process helper) async {
    // `cat` ends at the end of its input, and systemd-inhibit with it. A
    // helper already gone may refuse the close; the kill covers both.
    try {
      await helper.stdin.close();
    } on IOException {
      // Nothing left to read it.
    }
    helper.kill();
  }

  static const _esContinuous = 0x80000000;
  static const _esSystemRequired = 0x00000001;

  /// `SetThreadExecutionState` from kernel32; 0 when Windows refused.
  static int _windowsState(int flags) =>
      DynamicLibrary.open('kernel32.dll')
          .lookupFunction<Uint32 Function(Uint32), int Function(int)>('SetThreadExecutionState')(flags);
}
