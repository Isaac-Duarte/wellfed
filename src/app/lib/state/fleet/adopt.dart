import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/feeder_ports.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/server/server.dart';

/// How one feeder's adoption onto the server stands.
sealed class AdoptStatus {
  const AdoptStatus();
}

/// Asked, and no answer yet.
final class AdoptRunning extends AdoptStatus {
  /// An adoption on its way.
  const AdoptRunning();
}

/// The server holds it, and this phone's key is the account's.
final class AdoptDone extends AdoptStatus {
  /// An adoption that went through.
  const AdoptDone();
}

/// The server holds it, but this phone's key is not the account's yet: a
/// claim tries again.
final class AdoptUnclaimed extends AdoptStatus {
  /// Adopted, its key not yet claimed.
  const AdoptUnclaimed(this.error);

  /// Why the claim did not go through.
  final ServerError error;
}

/// The answer was lost, and the server may still be adopting it, or the
/// server is busy with another adoption of it: a check reads the server's
/// list, and sends nothing new.
final class AdoptUnsettled extends AdoptStatus {
  /// An adoption not known to have gone through or not.
  const AdoptUnsettled(this.error);

  /// How it was left: [ServerErrorKind.outcomeUnknown] while the server
  /// still looks for the feeder, [ServerErrorKind.feederBusy] while another
  /// adoption holds it.
  final ServerError error;
}

/// The feeder made no invite, so nothing went to the server.
final class AdoptNoInvite extends AdoptStatus {
  /// The feeder did not answer the invite.
  const AdoptNoInvite();
}

/// The server did not adopt it.
final class AdoptFailed extends AdoptStatus {
  /// An adoption the server refused.
  const AdoptFailed(this.error);

  /// What the server said.
  final ServerError error;
}

/// The feeders this phone is putting on the owner's server, by identity.
/// Every step goes through the router in Rust; a lost answer is never sent
/// again from here, only checked.
class AdoptFlow extends ChangeNotifier {
  /// Adoptions through `_ports`, refreshing `_server`'s list after each.
  AdoptFlow(this._ports, this._server);

  final FeederPorts _ports;
  final HomeServer _server;
  bool _disposed = false;

  /// Each feeder's adoption, once one is asked for.
  Map<String, AdoptStatus> get state => _state;
  Map<String, AdoptStatus> _state = const {};

  /// Adopt `feeder`; a feeder left unsettled is only checked, and one left
  /// unclaimed only claimed.
  Future<void> adopt(KnownFeeder feeder) => switch (_state[feeder.identity]) {
    AdoptRunning() || AdoptDone() => Future.value(),
    AdoptUnsettled() => _step(feeder.identity, () => _ports.checkAdoption(feeder.identity)),
    AdoptUnclaimed() => _step(feeder.identity, () => _ports.claimPhone(feeder.identity)),
    _ => _step(feeder.identity, () => _ports.adopt(feeder)),
  };

  Future<void> _step(String identity, Future<Adoption> Function() step) async {
    _set(identity, const AdoptRunning());
    final Adoption adoption = await step();
    if (_disposed) return;
    _set(identity, switch (adoption) {
      Adoption_Adopted() => const AdoptDone(),
      Adoption_Unclaimed(:final ServerError error) => AdoptUnclaimed(error),
      Adoption_Unsettled(:final ServerError error) => AdoptUnsettled(error),
      Adoption_NoInvite() => const AdoptNoInvite(),
      Adoption_Failed(:final ServerError error) => AdoptFailed(error),
    });
    if (adoption is Adoption_Adopted || adoption is Adoption_Unclaimed) await _server.refresh();
  }

  void _set(String identity, AdoptStatus status) {
    _state = Map.unmodifiable({..._state, identity: status});
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
