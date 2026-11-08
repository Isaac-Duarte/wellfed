import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';

/// A backup this phone keeps, as Settings lists it.
final class KeptCopy {
  /// The copy at `path` of feeder `identity`, called `name` on this phone.
  const KeptCopy({required this.identity, required this.path, this.name});

  /// The feeder it was read from, lowercase hex.
  final String identity;

  /// Where it is.
  final String path;

  /// The feeder's name on this phone; `null` for one it no longer lists,
  /// forgotten or given a new identity by a reset.
  final String? name;

  /// What the list calls it: its name, or the start of its identity, the
  /// fingerprint the feeder logs.
  String get label => name ?? 'Feeder ${identity.substring(0, identity.length < 16 ? identity.length : 16)}';
}

/// The backups found, or why they could not be listed.
final class KeptCopies {
  /// `copies`, and the `problem` reading them, if any.
  const KeptCopies(this.copies, {this.problem});

  /// Each copy, the feeders this phone lists first.
  final List<KeptCopy> copies;

  /// Why the backups folder could not be read.
  final String? problem;
}

/// The backups of the original firmware this phone keeps: every one in its
/// backups folder, including those of feeders it no longer lists.
class BackupsFlow extends ChangeNotifier {
  /// Backups, named from the feeders in `_book`.
  BackupsFlow(this._book, this._ports);

  final FeederBook _book;
  final BackupPorts _ports;
  bool _disposed = false;

  /// What was found.
  KeptCopies get state => _state;
  KeptCopies _state = const KeptCopies([]);

  /// Read what this phone keeps.
  Future<void> load() async {
    KeptCopies found;
    try {
      final Map<String, String> names = {for (final KnownFeeder feeder in _book.feeders) feeder.identity: feeder.name};
      final List<KeptCopy> all = [
        for (final KeptBackup kept in await _ports.kept())
          KeptCopy(identity: kept.identity, path: kept.path, name: names[kept.identity]),
      ];
      // Listed feeders first, each group in the folder's order.
      found = KeptCopies(
        List.unmodifiable([...all.where((copy) => copy.name != null), ...all.where((copy) => copy.name == null)]),
      );
    } on BridgeError catch (error) {
      found = KeptCopies(const [], problem: "Couldn't read the backups: ${error.message}.");
    }
    if (_disposed) return;
    _state = found;
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
