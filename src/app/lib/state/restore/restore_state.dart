import 'package:wellfed/src/rust/api/restore.dart';
import 'package:wellfed/state/install/install_state.dart' show CheckState;

/// What became of a feeder after an uncertain end.
enum RestoreLook {
  /// Still looking.
  looking,

  /// A session read it idle: Wellfed runs, as it did.
  wellfed,

  /// Seen on the network, but never read idle: it may be mid-install, about
  /// to restart, or a stale sighting.
  sighted,

  /// A feeder nearby advertises Bluetooth setup. Nothing says which: it may
  /// be this one with its people cleared, or another.
  setup,

  /// Neither in time: probably on the original firmware.
  nothing,
}

/// How one restore stands. The steps themselves are Rust's: this follows what
/// the run reports.
sealed class RestoreState {
  /// A state of a restore.
  const RestoreState();

  /// The offer may be out: leaving would end the run at nothing the owner
  /// could act on, so the screen offers no way back until it ends.
  bool get committed => false;

  /// A new run may go again: nothing was offered.
  bool get mayRetry => false;
}

/// Reading what this phone kept.
final class RestoreLoading extends RestoreState {
  /// The state before anything is known.
  const RestoreLoading();
}

/// No copy of stock to put back: pick one from a file.
final class RestoreNoBackup extends RestoreState {
  /// No usable copy; [notStock] says why the last one looked at failed.
  const RestoreNoBackup({this.notStock});

  /// Why the last copy looked at is not stock.
  final String? notStock;
}

/// The facts, the warnings and the owner's word.
final class RestoreBefore extends RestoreState {
  /// A copy ready to go back.
  const RestoreBefore({
    required this.version,
    required this.backupDate,
    required this.picked,
    required this.understood,
  });

  /// The stock version of the copy.
  final String? version;

  /// When the copy was made.
  final DateTime? backupDate;

  /// The owner picked this copy; it was not kept during the install.
  final bool picked;

  /// The owner has given their word.
  final bool understood;
}

/// The press, the sending, the restart.
final class RestoreRunning extends RestoreState {
  /// A run under way.
  const RestoreRunning({
    required this.pressDispense,
    required this.pressing,
    required this.sending,
    required this.written,
    required this.restarting,
    required this.sessionDropped,
    required this.holdRefused,
  });

  /// Waiting on the owner at the panel: Dispense enrolls the restore's key.
  final bool pressDispense;

  /// The press step.
  final CheckState pressing;

  /// The sending step.
  final CheckState sending;

  /// How much of the image went across, from 0 to 1.
  final double written;

  /// The restart step.
  final CheckState restarting;

  /// The session dropped and the run is getting it back.
  final bool sessionDropped;

  /// Android refused the foreground service, so only staying on screen keeps
  /// the run alive.
  final bool holdRefused;

  @override
  bool get committed => sending != CheckState.waiting;

  /// What the run waits for now, for a long wait.
  String get waitWords => sending == CheckState.waiting
      ? 'Waiting for the feeder to answer on your Wi-Fi.'
      : 'Waiting for the feeder. It restarts on the way, which can take a few '
            'minutes.';
}

/// Bank 1 is activated: the feeder restarts into the vendor's firmware.
final class RestoreDone extends RestoreState {
  /// A finished restore of [version].
  const RestoreDone({required this.version});

  /// The stock version going back on.
  final String? version;
}

/// The feeder stopped answering once the offer was out.
final class RestoreUncertain extends RestoreState {
  /// An end nobody can state yet.
  const RestoreUncertain({required this.why, required this.detail, required this.look});

  /// Why the end is not known.
  final UncertainWhy why;

  /// What the run said.
  final String? detail;

  /// What the look after it has found.
  final RestoreLook look;
}

/// The run stopped, as [stop] says.
final class RestoreStopped extends RestoreState {
  /// A stopped run.
  const RestoreStopped({required this.stop, required this.detail});

  /// How it stopped.
  final RestoreStop stop;

  /// What the run said.
  final String? detail;

  @override
  bool get mayRetry => switch (stop) {
    RestoreStop.noPress ||
    RestoreStop.enrollmentUnknown ||
    RestoreStop.keyRefused ||
    RestoreStop.keyGone ||
    RestoreStop.installRunning ||
    RestoreStop.cancelUnknown ||
    RestoreStop.notAnswering ||
    RestoreStop.cancelled => true,
    _ => false,
  };
}
