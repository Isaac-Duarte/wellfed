import 'package:flutter/foundation.dart';

import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/store.dart';

/// The app's files that did not read at this start and were set aside: kept
/// under another name, never deleted, and replaced with empty ones. The
/// client key (secure storage) and the stock backups live elsewhere and are
/// untouched.
final class StoreNotice {
  const StoreNotice._(this.files);

  /// A notice for `setAside`, or `null` when nothing was set aside.
  static StoreNotice? of(Iterable<SetAside?> setAside) {
    final List<SetAside> files = List.unmodifiable(setAside.nonNulls);
    return files.isEmpty ? null : StoreNotice._(files);
  }

  /// Each file set aside, once.
  final List<SetAside> files;

  /// The record of writes that may have run was among them. A feed it held
  /// may have run, so no feed goes until the owner has looked at the bowls.
  bool get holdsFeeds => files.any((file) => file.kind == StoreKind.pending);
}

/// The notice the start owes the owner, until it is read. The record of
/// writes stays set aside across starts until the owner says the bowls are
/// checked, since the router holds every feed back until then.
class StoreNoticeFlow extends ChangeNotifier {
  /// The notice for the app's file as the start opened it (`app`) and the
  /// router's record of writes, read from `router`.
  StoreNoticeFlow({SetAside? app, PathRouter? router})
    : _router = router,
      _state = StoreNotice.of([app, router?.setAside()]);

  final PathRouter? _router;
  StoreNotice? _state;

  /// What is still to be said; `null` once read, or with nothing set aside.
  StoreNotice? get state => _state;

  /// The owner read it, and with [StoreNotice.holdsFeeds] looked in every
  /// feeder's bowl: feeds go again. It stays when the router could not write
  /// that down, so feeds still wait.
  void read() {
    final StoreNotice? notice = _state;
    if (notice == null) return;
    if (notice.holdsFeeds) {
      try {
        _router?.bowlsChecked();
      } on BridgeError catch (error) {
        debugPrint('the checked bowls could not be written down, so feeds still wait: ${error.message}');
        return;
      }
    }
    _state = null;
    notifyListeners();
  }
}
