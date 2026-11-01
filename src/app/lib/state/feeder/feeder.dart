import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/platform/feeder_ports.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart' show ImageCheck;
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/restore/restore.dart';
import 'package:wellfed/state/update/update.dart';

/// One known feeder for the Home card: keeps a session open while the app
/// runs, reconnecting with backoff, and runs feeds through it. Every feed
/// goes through the device API like any other client's; this holds no
/// feeding logic beyond showing what the feeder said.
class FeederController extends ChangeNotifier {
  /// Starts serving `known` at once, and until disposed.
  FeederController(this._book, this._ports, KnownFeeder known) : _state = FeederState(known: known) {
    unawaited(_serve());
  }

  static const _locateWait = Duration(seconds: 8);
  static const _retryFirst = Duration(seconds: 3);
  static const _retryMost = Duration(seconds: 30);

  final FeederBook _book;
  final FeederPorts _ports;

  /// This feeder's update: it outlives the firmware screen, so coming back shows
  /// the run as it stands and can never start a second one.
  UpdateFlow get update => _update ??= UpdateFlow(this, _ports);
  UpdateFlow? _update;

  /// All that is known of this feeder.
  FeederState get state => _state;
  FeederState _state;

  FeederSession? _session;
  bool _disposed = false;

  /// Runs out when a running feed's result is overdue.
  Timer? _feedDue;

  /// A restore of this feeder to its original firmware.
  RestoreFlow newRestore() {
    final KnownFeeder known = _state.known;
    return RestoreFlow(
      _ports.restore,
      identity: identityBytes(hex: known.identity),
      feederName: known.name,
      address: known.address,
    );
  }

  /// The image this app carries, for the firmware screen to offer.
  Future<ImageCheck> updateImage() => _ports.updateImage();

  /// Feed `portions` once. The feeder's answer and the feed's end arrive
  /// through the session.
  void feedNow(int portions) {
    final FeederSession? session = _session;
    if (session == null || _feed.active) return;
    final sent = DateTime.now();
    _setFeed(FeedSending(portions: portions, sentAt: sent));
    try {
      session.feed(portions: portions);
    } on BridgeError {
      // The session had already ended; the request never went out.
      _setFeed(FeedNotSent(portions: portions, sentAt: sent));
    }
  }

  /// Ask the feeder to stop the running feed. This never says it stopped: only
  /// the feed's end does, and a request that can't go out says so.
  void cancelFeed() {
    final FeedState running = _feed;
    if (running is! FeedRunning) return;
    final FeederSession? session = _session;
    var sent = session != null;
    try {
      session?.cancelFeed(feed: running.feed);
    } on BridgeError {
      sent = false;
    }
    _setFeed(
      FeedRunning(
        feed: running.feed,
        portions: running.portions,
        sentAt: running.sentAt,
        cancelAsked: sent,
        cancelUnconfirmed: !sent,
      ),
    );
  }

  /// Put a finished feed away.
  void dismissFeed() => _setFeed(const FeedIdle());

  void Function(int dropped, List<String> lines)? _log;

  /// Hear the record lines a tail pushes, while a screen is showing them; null
  /// stops.
  // ignore: use_setters_to_change_properties, it subscribes, and nothing reads it back as a property.
  void hearLog(void Function(int dropped, List<String> lines)? listener) => _log = listener;

  /// A screen is hearing the record lines.
  bool get hearingLog => _log != null;

  /// A client by its fingerprint, as events name it.
  String clientName(String fingerprint) {
    for (final ClientView client in _state.clients) {
      if (client.fingerprint == fingerprint) {
        return client.you ? 'This phone' : client.label;
      }
    }
    return 'A phone or server';
  }

  /// Read who the feeder lists.
  Future<void> loadClients() async {
    final FeederReply reply = await ask(const FeederRequest.listClients());
    if (reply is FeederReply_Clients) _setState(_state.copyWith(clients: reply.clients));
  }

  /// Take this feeder off this phone.
  Future<void> forgetHere() => _book.forget(_state.known.identity);

  /// One request to the feeder. A write goes out once and is never sent
  /// again; the reply says how it ended, an unknown end included.
  Future<FeederReply> ask(FeederRequest request) async {
    final FeederSession? session = _session;
    if (session == null) return const FeederReply.notSent();
    return session.ask(request: request);
  }

  Future<void> _serve() async {
    Duration retry = _retryFirst;
    while (!_disposed) {
      final String? address = await _address();
      if (_disposed) return;
      if (address == null) {
        _setState(_state.copyWith(connection: Connection.offline, error: () => ErrorKind.connect));
      } else {
        _set(Connection.connecting);
        final bool opened = await _run(address);
        if (opened) retry = _retryFirst;
      }
      if (_disposed) return;
      await Future<void>.delayed(retry);
      retry = retry * 2 > _retryMost ? _retryMost : retry * 2;
    }
  }

  /// The last address, or NSD's when there is none or it stopped working.
  Future<String?> _address() async {
    if (_state.connection != Connection.offline && _state.known.address != null) {
      return _state.known.address;
    }
    _set(Connection.looking);
    return await _ports.locate(_state.known.identity, _locateWait) ?? _state.known.address;
  }

  /// One session until it closes; whether it opened.
  Future<bool> _run(String address) async {
    final FeederSession session;
    try {
      session = _ports.open(_state.known.identity, address, syncClock: _book.clockDue(_state.known.identity));
    } on BridgeError catch (failed) {
      _setState(_state.copyWith(connection: Connection.offline, error: () => failed.kind));
      return false;
    }
    _session = session;
    _setState(_state.copyWith(cardFresh: false));

    var opened = false;
    await for (final SessionUpdate update in session.run()) {
      if (_disposed) {
        session.close();
        break;
      }
      switch (update) {
        case SessionUpdate_Opened(:final String build, role: final ClientRole role):
          unawaited(_ports.answered(_state.known.identity, build));
          opened = true;
          _setState(
            _state.copyWith(
              connection: Connection.online,
              role: role,
              error: () => null,
              history: const [],
              historyRead: false,
            ),
          );
          try {
            await _book.answeredAt(_state.known.identity, address);
            final KnownFeeder known = _state.known;
            _setState(
              _state.copyWith(
                known: _book.feeders.firstWhere((f) => f.identity == known.identity, orElse: () => known),
              ),
            );
          } on BridgeError {
            // Only a hint for the next start; NSD finds the feeder anyway.
          }
        case SessionUpdate_Status(card: final Card card):
          _setState(_state.copyWith(card: card, cardAt: DateTime.now(), cardFresh: true));
        case SessionUpdate_FeedStarted(:final int feed, :final int portions, :final int dueSeconds):
          _feedDue?.cancel();
          _feedDue = Timer(Duration(seconds: dueSeconds), _feedOverdue);
          _setFeed(FeedRunning(feed: feed, portions: portions, sentAt: _feed.sentAt));
        case SessionUpdate_FeedRefused(:final FeedRefusal refusal):
          _setFeed(FeedRefused(refusal: refusal, portions: _feed.portions, sentAt: _feed.sentAt));
        case SessionUpdate_FeedUnanswered():
          _setFeed(FeedUnanswered(portions: _feed.portions, sentAt: _feed.sentAt));
        case SessionUpdate_FeedEnded(:final FeedResult result):
          _feedEnded(result);
        case SessionUpdate_CancelUnconfirmed():
          if (_feed case FeedRunning(:final int feed, :final int portions, :final DateTime? sentAt)) {
            _setFeed(FeedRunning(feed: feed, portions: portions, sentAt: sentAt, cancelUnconfirmed: true));
          }
        case SessionUpdate_ClockSynced():
          _book.clockSynced(_state.known.identity);
        case SessionUpdate_ClockNotSynced():
          // Still due: the next session tries again.
          break;
        case SessionUpdate_History(:final List<HistoryEntry> entries, :final bool caughtUp):
          _setState(
            _state.copyWith(history: [..._state.history, ...entries], historyRead: _state.historyRead || caughtUp),
          );
          // A feed that ended while the link was down, or before this
          // session's history was read, only ever arrives here.
          for (final entry in entries) {
            if (entry.body case HistoryBody_Fed(:final FeedResult result)) _feedEnded(result);
          }
        case SessionUpdate_Log(:final int dropped, :final List<String> lines):
          _log?.call(dropped, lines);
        case SessionUpdate_Closed(error: final BridgeError? error):
          _setState(_state.copyWith(error: () => error?.kind));
      }
    }

    _session = null;
    _setState(_state.copyWith(cardFresh: false));
    // The session reports every feed it sent as started, refused or
    // unanswered before it closes, so one still sending never went out.
    switch (_feed) {
      case FeedSending(:final int portions, :final DateTime? sentAt):
        _setFeed(FeedNotSent(portions: portions, sentAt: sentAt));
      case FeedRunning(:final int feed, :final int portions, :final DateTime? sentAt):
        _setFeed(FeedAdrift(feed: feed, by: FeedLoss.link, portions: portions, sentAt: sentAt));
      case _:
        break;
    }
    if (!_disposed) _setState(_state.copyWith(connection: Connection.offline));
    return opened;
  }

  /// A feed's end, which settles ours only when it carries our number: an
  /// unanswered feed has none, and the next end may be a meal or another
  /// phone's. It replaces a lost feed too, since it is what really happened.
  void _feedEnded(FeedResult result) {
    final FeedState feed = _feed;
    final int? ours = switch (feed) {
      FeedRunning(:final int feed) => feed,
      FeedAdrift(:final int feed) => feed,
      _ => null,
    };
    if (ours != result.feed) return;

    // The feeder's record says when, if its clock can be trusted. A lost feed's
    // end may be old, so the phone's clock can't stand in for that one.
    final PlatformInt64? recorded = (_state.card?.clockSet ?? false) ? result.atMs : null;
    _setFeed(
      FeedEnded(
        result: result,
        sentAt: feed.sentAt,
        endedAt: recorded != null
            ? DateTime.fromMillisecondsSinceEpoch(recorded)
            : feed is FeedRunning
            ? DateTime.now()
            : null,
      ),
    );
  }

  /// Nothing has come of a running feed past the longest the feeder can take.
  void _feedOverdue() {
    if (_feed case FeedRunning(:final int feed, :final int portions, :final DateTime? sentAt)) {
      _setFeed(FeedAdrift(feed: feed, by: FeedLoss.overdue, portions: portions, sentAt: sentAt));
    }
  }

  FeedState get _feed => _state.feed;

  void _setFeed(FeedState next) {
    if (next is! FeedRunning) {
      _feedDue?.cancel();
      _feedDue = null;
    }
    _setState(_state.copyWith(feed: next));
  }

  void _set(Connection next) => _setState(_state.copyWith(connection: next));

  void _setState(FeederState next) {
    _state = next;
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _feedDue?.cancel();
    _session?.close();
    _update?.dispose();
    super.dispose();
  }
}
