part of 'feeder.dart';

/// The phone's own session with the feeder: one at a time, each attached to
/// the router before it runs, so the router hears its history from the
/// start and nothing reaches the feeder around it.
extension FeederSessionRun on FeederController {
  /// One session until it closes; whether it opened.
  Future<bool> _run(String address) async {
    final FeederSession session;
    try {
      session = _ports.open(_state.known.identity, address, syncClock: _book.clockDue(_state.known.identity));
    } on BridgeError catch (failed) {
      _sessionIs(Connection.offline, error: failed.kind);
      return false;
    }
    _ports.router?.attach(session: session);
    _session = session;
    _setState(_state.copyWith(cardFresh: false));

    var opened = false;
    await for (final SessionUpdate update in session.run()) {
      if (_disposed) {
        session.close();
        break;
      }
      switch (update) {
        case SessionUpdate_Opened(:final String build, role: final ClientRole role, :final Skew skew):
          unawaited(_ports.answered(_state.known.identity, build));
          opened = true;
          _connection = Connection.online;
          _setState(
            _state.copyWith(
              role: role,
              skew: () => skew,
              error: () => null,
              // A new session reads the card again.
              cardFresh: false,
              history: const [],
              historyRead: false,
              // A new session may meet a feeder another client re-weighed or
              // gave a new clip, or one whose store was erased.
              call: () => null,
              calibration: () => null,
            ),
          );
          // The owner's address may be a VPN's route, which updates and
          // restores can't use, so only where NSD found the feeder is kept.
          if (address != _state.known.manualAddress) {
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
          }
        case SessionUpdate_Status(card: final Card card):
          _setState(_state.copyWith(card: card, cardAt: clock.now(), cardFresh: true));
        case SessionUpdate_FeedStarted(:final int feed, :final int portions, :final int dueSeconds):
          _feedStarted(feed, portions, dueSeconds);
        case SessionUpdate_FeedRefused(:final FeedRefusal refusal):
          if (_feed is FeedSending) {
            _setFeed(FeedRefused(refusal: refusal, portions: _feed.portions, sentAt: _feed.sentAt));
          }
        case SessionUpdate_FeedUnanswered():
          // The router's answer for the same feed brings its key.
          if (_feed is FeedSending || _feed is FeedRunning) {
            _setFeed(FeedUnanswered(portions: _feed.portions, sentAt: _feed.sentAt));
          }
        case SessionUpdate_FeedEnded(:final FeedResult result):
          _feedEnded(result);
        case SessionUpdate_CancelUnconfirmed():
          if (_feed case final FeedRunning running) _setFeed(running.copyCancel(unconfirmed: true));
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
    // A feed running when the link went is adrift.
    if (_feed case FeedRunning(:final int feed, :final int portions, :final DateTime? sentAt)) {
      _setFeed(FeedAdrift(feed: feed, by: FeedLoss.link, portions: portions, sentAt: sentAt));
    }
    // A feeder of another major refuses the session at the handshake: it
    // is reachable all the same.
    if (!_disposed) {
      _sessionIs(_state.error == ErrorKind.version ? Connection.otherVersion : Connection.offline);
    }
    return opened;
  }

  /// The feeder reserved a feed of this phone's as `feed`: from the session's
  /// stream, or the router's answer. Both may say it, in either order, and
  /// the second changes nothing. Past `dueSeconds` a feeder that stayed on
  /// has ended it.
  void _feedStarted(int feed, int portions, int dueSeconds, {String? key}) {
    final FeedState now = _feed;
    switch (now) {
      case FeedRunning(feed: final int running) when running == feed:
        _setFeed(
          FeedRunning(
            feed: feed,
            portions: now.portions,
            sentAt: now.sentAt,
            cancelAsked: now.cancelAsked,
            cancelUnconfirmed: now.cancelUnconfirmed,
            key: key ?? now.key,
          ),
        );
      case FeedEnded(:final FeedResult result) when result.feed == feed:
        // Its end came first.
        break;
      case FeedSending() || FeedUnanswered() || FeedIdle():
        _feedDue?.cancel();
        _feedDue = Timer(Duration(seconds: dueSeconds), _feedOverdue);
        _setFeed(FeedRunning(feed: feed, portions: portions, sentAt: now.sentAt, key: key));
      case _:
        break;
    }
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

  /// Nothing has come of a running feed past the longest the feeder can take:
  /// the owner looks at the bowl.
  void _feedOverdue() {
    if (_feed case FeedRunning(:final int feed, :final int portions, :final DateTime? sentAt)) {
      _setFeed(FeedAdrift(feed: feed, by: FeedLoss.overdue, portions: portions, sentAt: sentAt));
    }
  }
}

extension on FeedRunning {
  /// The same feed, its stop asked for (`unconfirmed`: and not confirmed).
  FeedRunning copyCancel({bool asked = false, bool unconfirmed = false}) => FeedRunning(
    feed: feed,
    portions: portions,
    sentAt: sentAt,
    cancelAsked: asked,
    cancelUnconfirmed: unconfirmed,
    key: key,
  );
}
