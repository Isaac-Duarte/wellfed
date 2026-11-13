import 'dart:io';

import 'package:flutter/foundation.dart';

import 'package:wellfed/platform/host.dart';
import 'package:wellfed/platform/secure_storage.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/one_at_a_time.dart';
import 'package:wellfed/state/fleet/store_notice.dart';

/// The feeders this phone knows, and what the flows that add one need of
/// the app's file.
abstract interface class FeederBook implements Listenable {
  /// How feeders list this phone; empty until the owner names it.
  String get label;

  /// The feeders this phone knows.
  List<KnownFeeder> get feeders;

  /// Save how feeders list this phone.
  Future<void> setLabel(String label);

  /// Which theme the app draws.
  Appearance get appearance;

  /// Save which theme the app draws.
  Future<void> setAppearance(Appearance appearance);

  /// Add `feeder`, or replace the one with its identity. With `paired`, this
  /// phone just became its owner, so its next session gives it this phone's
  /// clock. With `keepName`, a feeder already known keeps the name the owner
  /// gave it.
  Future<void> remember(KnownFeeder feeder, {bool paired = false, bool keepName = false});

  /// Keep where `identity` last answered, so the next start tries it before
  /// NSD has looked.
  Future<void> answeredAt(String identity, String address);

  /// Dial `identity` at `address` (`ip:port`) instead of looking for it with
  /// NSD, or look again when `null`. For a feeder reached over a VPN's route
  /// to the home network, where NSD's multicast never arrives.
  Future<void> setManualAddress(String identity, String? address);

  /// Remove `identity` from this phone. It leaves the feeder's list as it
  /// is: only an owner's revocation takes a key off a feeder.
  Future<void> forget(String identity);

  /// Call `identity` `name` on this phone. The name is this phone's alone:
  /// the feeder keeps none, so another phone keeps its own.
  Future<void> rename(String identity, String name);

  /// Whether this phone owes `identity` its clock, having just paired.
  bool clockDue(String identity);

  /// This phone gave `identity` its clock, so it no longer owes it.
  void clockSynced(String identity);
}

/// Why the pins file (`pending.json`) did not open, so the app sends no
/// request at all. A file that opens but does not read is not one of these:
/// it is set aside, and [AppState.notice] says so.
enum PinsProblem {
  /// Another copy of the app, in another process, holds it.
  inUse,

  /// The phone's storage failed to read it: it may read at the next start.
  unavailable,
}

/// How the app's start went.
sealed class AppStart {
  /// For the two outcomes.
  const AppStart();
}

/// The app's state, ready.
final class AppStarted extends AppStart {
  /// Started on `app`.
  const AppStarted(this.app);

  /// The phone's key, its feeders and its router.
  final AppState app;
}

/// The phone's key or the app's file could not be read at all: not their
/// contents, which are set aside, but the storage under them.
final class AppFailed extends AppStart {
  /// Failed, saying `message`.
  const AppFailed(this.message);

  /// What failed, for a support request.
  final String message;
}

/// The phone's identity and the feeders it knows. The client secret lives
/// in [secureStorage]; everything else in the app's own file, which Rust
/// reads and writes.
class AppState extends ChangeNotifier implements FeederBook {
  AppState._(
    this.key,
    this._store,
    this._path,
    (FeederRouter?, PinsProblem?) opened,
    SetAside? storeSetAside, {
    SetAside? alertsSetAside,
  }) : router = opened.$1,
       pinsProblem = opened.$2 {
    // Clearing the notice rewrites the app's file, so it waits its turn
    // behind any change still being written, as each change does.
    notice = StoreNoticeFlow(
      app: storeSetAside,
      alerts: alertsSetAside,
      router: opened.$1,
      appRead: () => _writing.run(() async => storeNoticeRead(path: _path)),
      alertsRead: () => alertsNoticeRead(dir: File(_path).parent.path),
    );
  }

  /// The state of a phone with `key` and `store`, saved at `path`, its
  /// router `router` (none: the pins did not open) and the app's file set
  /// aside as `storeSetAside`, for a test of the whole app.
  @visibleForTesting
  AppState.of({
    required ClientKey key,
    required AppStore store,
    required String path,
    FeederRouter? router,
    SetAside? storeSetAside,
  }) : this._(key, store, path, (router, router == null ? PinsProblem.unavailable : null), storeSetAside);

  /// This phone's key.
  final ClientKey key;

  /// The one router, whose pins (`pending.json`) live beside the app's
  /// file; `null` when they could not be opened, and then the app sends no
  /// request at all rather than an unpinned write.
  final FeederRouter? router;

  /// Why [router] is `null`, for the owner to see.
  final PinsProblem? pinsProblem;
  final String _path;
  AppStore _store;

  /// The app's files this start set aside, until the owner has read what
  /// that means.
  late final StoreNoticeFlow notice;

  /// Feeders that paired since the app started, whose clock this phone has
  /// not set yet. In memory: a feeder with no clock is set whenever a
  /// session opens, whatever this holds.
  final _clockDue = <String>{};

  final _writing = OneAtATime();

  /// Start the app: [load], or why it could not.
  static Future<AppStart> start() async {
    try {
      return AppStarted(await load());
    } on Object catch (error) {
      // Any failure here would otherwise leave a blank screen: say it.
      debugPrint('the app could not start: $error');
      return AppFailed(error is BridgeError ? error.message : '$error');
    }
  }

  /// Read this phone's key and file, making a key the first time. A file
  /// that does not read is set aside ([notice]); only the storage itself
  /// failing, or a file a newer app wrote, throws.
  static Future<AppState> load() async {
    // Only a read that finds no entry is a first start. A read that fails
    // throws (the storage never resets on error), so a key the Keystore
    // could not open this time is never replaced by a new one.
    final String? saved = await secureStorage.read(key: clientSecretName);
    final ClientKey key;
    if (saved == null) {
      key = ClientKey.generate();
      // The store's hex spelling, as for feeder identities: 32 bytes.
      await secureStorage.write(
        key: clientSecretName,
        value: identityHex(identity: key.secret()),
      );
    } else {
      key = ClientKey.restore(secret: identityBytes(hex: saved));
    }

    final Directory dir = await appDataDirectory();
    final path = '${dir.path}/wellfed.json';
    final OpenedStore opened = await openStore(path: path);
    return AppState._(
      key,
      opened.store,
      path,
      _openRouter('${dir.path}/pending.json'),
      opened.setAside,
      // Kept in the alert file until read: a catch-up in the background may
      // have been the one to set it aside.
      alertsSetAside: await alertsSetAside(dir: dir.path),
    );
  }

  /// The router on `pins`.
  static (FeederRouter?, PinsProblem?) _openRouter(String pins) {
    try {
      return (FeederRouter(pinsPath: pins), null);
    } on BridgeError catch (error) {
      debugPrint('the pending writes could not be opened, so nothing is sent: ${error.message}');
      return (null, error.kind == ErrorKind.inUse ? PinsProblem.inUse : PinsProblem.unavailable);
    }
  }

  @override
  String get label => _store.label;

  @override
  List<KnownFeeder> get feeders => _store.feeders;

  @override
  bool clockDue(String identity) => _clockDue.contains(identity);

  @override
  void clockSynced(String identity) => _clockDue.remove(identity);

  @override
  Appearance get appearance => _store.appearance;

  @override
  Future<void> setAppearance(Appearance appearance) =>
      _update((store) => AppStore(label: store.label, feeders: store.feeders, appearance: appearance));

  @override
  Future<void> setLabel(String label) =>
      _update((store) => AppStore(label: label, feeders: store.feeders, appearance: store.appearance));

  @override
  Future<void> remember(KnownFeeder feeder, {bool paired = false, bool keepName = false}) {
    if (paired) _clockDue.add(feeder.identity);
    return _update((store) {
      final Iterable<KnownFeeder> known = store.feeders.where((f) => f.identity == feeder.identity);
      return AppStore(
        label: store.label,
        feeders: [
          for (final other in store.feeders)
            if (other.identity != feeder.identity) other,
          KnownFeeder(
            identity: feeder.identity,
            name: keepName && known.isNotEmpty ? known.first.name : feeder.name,
            address: feeder.address,
            // A flow that adds a feeder again doesn't know the owner's
            // address, so the one they typed stays.
            manualAddress: feeder.manualAddress ?? known.firstOrNull?.manualAddress,
          ),
        ],
        appearance: store.appearance,
      );
    });
  }

  @override
  Future<void> forget(String identity) => _update(
    (store) => AppStore(
      label: store.label,
      feeders: [
        for (final feeder in store.feeders)
          if (feeder.identity != identity) feeder,
      ],
      appearance: store.appearance,
    ),
  );

  @override
  Future<void> rename(String identity, String name) => _update(
    (store) => AppStore(
      label: store.label,
      feeders: [
        for (final feeder in store.feeders)
          if (feeder.identity == identity)
            KnownFeeder(
              identity: feeder.identity,
              name: name,
              address: feeder.address,
              manualAddress: feeder.manualAddress,
            )
          else
            feeder,
      ],
      appearance: store.appearance,
    ),
  );

  @override
  Future<void> setManualAddress(String identity, String? address) => _update(
    (store) => AppStore(
      label: store.label,
      feeders: [
        for (final feeder in store.feeders)
          if (feeder.identity == identity)
            KnownFeeder(identity: feeder.identity, name: feeder.name, address: feeder.address, manualAddress: address)
          else
            feeder,
      ],
      appearance: store.appearance,
    ),
  );

  @override
  Future<void> answeredAt(String identity, String address) async {
    final Iterable<KnownFeeder> known = _store.feeders.where((f) => f.identity == identity);
    if (known.isEmpty || known.first.address == address) {
      return;
    }
    await remember(KnownFeeder(identity: identity, name: known.first.name, address: address), keepName: true);
  }

  /// Change the file from what it holds now, not from this copy of it: an
  /// install's run saves a feeder there from Rust, before Dart hears of it.
  /// One at a time, so two changes do not read the same file, and a save
  /// lands only on the file it was made from: one the run changed meanwhile
  /// is read again and the change made anew.
  Future<void> _update(AppStore Function(AppStore) change) => _writing.run(() async {
    while (true) {
      final AppStore read = await loadStore(path: _path);
      final AppStore store = change(read);
      if (await swapStore(path: _path, read: read, store: store)) {
        _store = store;
        notifyListeners();
        return;
      }
    }
  });
}
