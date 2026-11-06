import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';

/// The backups of the original firmware this phone keeps.
class BackupsFlow extends ChangeNotifier {
  /// Backups of the feeders in `_book`.
  BackupsFlow(this._book, this._ports);

  final FeederBook _book;
  final BackupPorts _ports;
  bool _disposed = false;

  /// Each feeder with a backup here, and where it is.
  List<(KnownFeeder, String)> get state => _state;
  List<(KnownFeeder, String)> _state = const [];

  /// Read what this phone keeps.
  Future<void> load() async {
    final kept = <(KnownFeeder, String)>[];
    for (final KnownFeeder feeder in _book.feeders) {
      if (await _ports.kept(feeder.identity) case final path?) kept.add((feeder, path));
    }
    if (_disposed) return;
    _state = List.unmodifiable(kept);
    notifyListeners();
  }

  /// Hand the backup at `path` to wherever the owner keeps files: why it
  /// could not be saved, or `null` when it was or the owner cancelled.
  Future<String?> share(String path) async {
    try {
      await _ports.share(path);
      return null;
    } on FileSystemException catch (error) {
      return "Couldn't save the backup: ${error.osError?.message ?? error.message}.";
    }
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
