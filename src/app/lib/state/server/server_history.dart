import 'package:flutter/foundation.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/session.dart';

/// A feeder's history as the owner's server kept it, longer than the feeder's
/// own ring holds.
@immutable
final class ServerHistoryState {
  /// Nothing read yet.
  const ServerHistoryState({this.entries = const [], this.reading = true, this.error});

  /// Every entry read so far, newest first.
  final List<HistoryEntry> entries;

  /// More pages are being read.
  final bool reading;

  /// What stopped the reading; `null` while it goes or once it is done.
  final ServerError? error;
}

/// The long history of one feeder: the server's pages of its current
/// history, oldest first by `after`, read to the newest and shown newest
/// first. Reading only; nothing reaches the feeder.
class ServerHistoryFlow extends ChangeNotifier {
  /// The history of server feeder `_feeder` on `_session`.
  ServerHistoryFlow(this._session, this._feeder);

  /// The most the server gives in one page.
  static const pageSize = 500;

  final ServerSession _session;
  final String _feeder;
  bool _disposed = false;

  /// What the screen shows.
  ServerHistoryState get state => _state;
  ServerHistoryState _state = const ServerHistoryState();

  /// Read every page, from the oldest to the newest.
  Future<void> load() async {
    final oldestFirst = <HistoryEntry>[];
    BigInt? after;
    while (!_disposed) {
      final ServerEventPage page;
      try {
        page = await _session.events(feeder: _feeder, after: after, limit: pageSize);
      } on ServerError catch (error) {
        _set(ServerHistoryState(entries: _state.entries, reading: false, error: error));
        return;
      }
      oldestFirst.addAll(page.entries.map((entry) => entry.entry));
      final BigInt? next = page.next;
      // A next that does not move on would read the same page for good.
      final bool done = next == null || next == after || page.entries.isEmpty;
      _set(ServerHistoryState(entries: List.unmodifiable(oldestFirst.reversed), reading: !done));
      if (done) return;
      after = next;
    }
  }

  void _set(ServerHistoryState next) {
    if (_disposed) return;
    _state = next;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
