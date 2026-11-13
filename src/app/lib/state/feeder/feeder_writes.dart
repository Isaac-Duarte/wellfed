part of 'feeder.dart';

/// Changes other than a feed that went out and never said how they ended: a
/// setting, a schedule, a call. The router holds each; nothing here sends
/// one again. The banner shows them until the owner puts them away.
extension FeederWrites on FeederController {
  /// The owner has looked, and puts `key` away.
  void putAwayWrite(String key) {
    try {
      _ports.router?.acknowledge(key: key);
    } on BridgeError {
      // Not an idempotency key: nothing the router holds.
    }
    _readUnconfirmed();
  }

  /// What the router holds of this feeder's changes now.
  void _readUnconfirmed() {
    final List<PendingWrite> held = [
      for (final PendingWrite pending in _ports.router?.pending() ?? const <PendingWrite>[])
        if (pending.feeder == _state.known.identity && pending.kind != WriteKind.feed && !pending.acknowledged) pending,
    ];
    if (listEquals(held, _state.unconfirmed)) return;
    _setState(_state.copyWith(unconfirmed: held));
  }
}
