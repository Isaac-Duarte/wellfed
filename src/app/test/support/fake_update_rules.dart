import 'package:wellfed/platform/feeder_ports.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/update.dart';

/// The update rules `wellfed_client::offer` decides, restated for screens
/// tested with no Rust. The smoke tests hold them to the real ones, case by
/// case, so this copy cannot drift from what the app runs.
mixin FakeUpdateRules implements FeederPorts {
  @override
  UpdateOffer updateOffer({
    required String build,
    required int version,
    String? running,
    required Skew skew,
    UpdateInfo? update,
  }) {
    if (running != null && running.toLowerCase() == build.toLowerCase()) return const UpdateOffer.current();
    if (skew == Skew.feederAhead) return const UpdateOffer.older();
    if (update == null) return const UpdateOffer.unread();
    if (version <= update.minVersion) return const UpdateOffer.older();
    return switch (update.bank0) {
      Bank0Check_Blocked(:final Bank0Blocked field0) => UpdateOffer.blocked(reason: field0),
      Bank0Check_Unknown() => const UpdateOffer.blocked(reason: Bank0Blocked.unknown),
      _ => UpdateOffer.newer(build: build, version: version),
    };
  }

  @override
  UnknownEnd unknownEnd({
    String? served,
    required int version,
    required UpdateInfo update,
    String? running,
    required bool ready,
  }) {
    if (update.phase != UpdatePhase.idle) return UnknownEnd.unknown;
    if (update.bank == 0) return UnknownEnd.decide;
    if (served == null || running == null || running.toLowerCase() != served.toLowerCase()) return UnknownEnd.unknown;
    return update.minVersion >= version || ready ? UnknownEnd.confirmed : UnknownEnd.unknown;
  }
}
