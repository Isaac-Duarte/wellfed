import 'package:wellfed/src/rust/api/update.dart';

/// How one feeder's update stands.
sealed class UpdateState {
  /// A state of the update.
  const UpdateState();

  /// Only the feeder can tell how a run went, so its end stays until it has.
  bool get endUnknown => false;
}

/// Nothing under way, so the firmware screen may offer one.
final class UpdateIdle extends UpdateState {
  /// The state with no update.
  const UpdateIdle();
}

/// This device may not update a feeder, so nothing is offered or sent.
final class UpdateUnavailable extends UpdateState {
  /// Refused from the start.
  const UpdateUnavailable();
}

/// A run under way.
sealed class UpdateRun extends UpdateState {
  /// A run with this much sent.
  const UpdateRun({
    this.written = 0,
    this.length = 0,
    this.reconnecting = false,
    this.cancelling = false,
    this.holdRefused = false,
  });

  /// Bytes the feeder has taken.
  final int written;

  /// Bytes in the image, once the feeder accepts it.
  final int length;

  /// The session dropped and the run is getting it back.
  final bool reconnecting;

  /// A stop was asked for and has not been answered.
  final bool cancelling;

  /// Android refused the foreground service, so only staying on screen keeps
  /// the run alive.
  final bool holdRefused;

  /// This run with the named parts changed.
  UpdateRun changed({int? written, int? length, bool? reconnecting, bool? cancelling, bool? holdRefused});
}

/// Sending the image.
final class UpdateSending extends UpdateRun {
  /// A run still sending.
  const UpdateSending({super.written, super.length, super.reconnecting, super.cancelling, super.holdRefused});

  @override
  UpdateSending changed({int? written, int? length, bool? reconnecting, bool? cancelling, bool? holdRefused}) =>
      UpdateSending(
        written: written ?? this.written,
        length: length ?? this.length,
        reconnecting: reconnecting ?? this.reconnecting,
        cancelling: cancelling ?? this.cancelling,
        holdRefused: holdRefused ?? this.holdRefused,
      );
}

/// The feeder activated the image and is restarting into it.
final class UpdateRestarting extends UpdateRun {
  /// A run past its last byte.
  const UpdateRestarting({super.written, super.length, super.reconnecting, super.cancelling, super.holdRefused});

  @override
  UpdateRestarting changed({int? written, int? length, bool? reconnecting, bool? cancelling, bool? holdRefused}) =>
      UpdateRestarting(
        written: written ?? this.written,
        length: length ?? this.length,
        reconnecting: reconnecting ?? this.reconnecting,
        cancelling: cancelling ?? this.cancelling,
        holdRefused: holdRefused ?? this.holdRefused,
      );
}

/// The run ended, as [end] says.
final class UpdateEnded extends UpdateState {
  /// An ended run.
  const UpdateEnded(this.end, {this.stillUnknown = false, this.cancelling = false});

  /// How it ended.
  final UpdateEnd end;

  /// A look at an unknown end settled nothing.
  final bool stillUnknown;

  /// A cancel of the trial, from the end screen, is out.
  final bool cancelling;

  @override
  bool get endUnknown => end is UpdateEnd_Uncertain || end is UpdateEnd_Unconfirmed;

  /// This end with the named parts changed.
  UpdateEnded changed({bool? stillUnknown, bool? cancelling}) =>
      UpdateEnded(end, stillUnknown: stillUnknown ?? this.stillUnknown, cancelling: cancelling ?? this.cancelling);
}
