part of 'feeder.dart';

/// This phone's feeds, through the path router: it pins each before it goes
/// and sends it once, on the owner's server or the phone's own session. One
/// that may have run stays pending on its own path; a check only reads that
/// path, and nothing here ever sends a feed again. A new feed waits until the
/// owner has looked at the bowl and put the uncertain one away.
extension FeederFeeds on FeederController {
  /// Feed `portions` once. The feeder's answer comes from the router, and the
  /// feed's end from the session's or the server's stream. With no router
  /// (its pins could not be opened) nothing goes.
  void feedNow(int portions) {
    if (_feed.active) return;
    final sent = DateTime.now();
    final PathRouter? router = _ports.router;
    if (router == null) {
      _setFeed(FeedNotSent(portions: portions, sentAt: sent, why: NotSentWhy.notPinned));
      return;
    }
    _setFeed(FeedSending(portions: portions, sentAt: sent));
    unawaited(_feedRouted(router, portions));
  }

  Future<void> _feedRouted(PathRouter router, int portions) async {
    final RoutedFeed routed = await router.feed(feeder: _state.known.identity, portions: portions);
    if (_disposed) return;

    final FeedState now = _feed;
    switch (routed) {
      case RoutedFeed_Started(:final String key, :final RoutePath path, :final int feed, :final int dueSeconds):
        _feedStarted(feed, portions, dueSeconds, key: key, path: path);
      case RoutedFeed_Refused(:final FeedRefusal refusal):
        if (now is FeedSending) _setFeed(FeedRefused(refusal: refusal, portions: now.portions, sentAt: now.sentAt));
      case RoutedFeed_Uncertain(:final PendingWrite pending):
        _uncertain(pending);
      case RoutedFeed_Settled(:final PendingWrite pending, :final Settlement settlement):
        _settled(pending, settlement, ownCall: true);
      case RoutedFeed_NotSent(:final NotSentReason reason, :final ServerError? error):
        if (now is FeedSending) {
          _setFeed(FeedNotSent(portions: now.portions, sentAt: now.sentAt, why: _notSentWhy(reason, error)));
        }
      case RoutedFeed_Held(:final PendingWrite pending):
        // An earlier feed may have run: the sheet asks about that one.
        if (now is FeedSending) _uncertain(pending);
    }
  }

  /// Ask the feeder to stop the running feed. This never says it stopped: only
  /// the feed's end does, and a request that can't go out says so.
  void cancelFeed() {
    final FeedState running = _feed;
    if (running is! FeedRunning) return;
    final PathRouter? router = _ports.router;
    if (router == null) {
      _setFeed(running.copyCancel(unconfirmed: true));
      return;
    }
    _setFeed(running.copyCancel(asked: true));
    unawaited(_cancelRouted(router, running.feed));
  }

  Future<void> _cancelRouted(PathRouter router, int feed) async {
    final RoutedReply routed = await router.cancelFeed(feeder: _state.known.identity, feed: feed);
    if (_disposed || routed.reply is FeederReply_Done) return;
    if (_feed case final FeedRunning running when running.feed == feed) {
      _setFeed(running.copyCancel(unconfirmed: true));
    }
  }

  /// Put a finished feed away. An uncertain one the owner has now looked at
  /// is put away in the router too, so a new feed may go.
  void dismissFeed() {
    if (_feed case FeedUnanswered(:final String key?)) {
      try {
        _ports.router?.acknowledge(key: key);
      } on BridgeError {
        // Not an idempotency key: nothing the router holds.
      }
    }
    _setFeed(const FeedIdle());
  }

  /// Ask an uncertain feed's own path what became of it. This only reads:
  /// the server's journal, or the feeder's history as the phone's own session
  /// read it. Nothing is sent, on either path.
  Future<void> checkFeed() async {
    final PathRouter? router = _ports.router;
    if (router == null) return;
    if (_feed case FeedUnanswered(:final String key?)) {
      final RetryOutcome outcome = await router.retry(key: key);
      if (_disposed) return;
      if (outcome case RetryOutcome_Settled(:final PendingWrite pending, :final Settlement settlement)) {
        _settled(pending, settlement);
      }
    }
  }

  /// What the router said of this feeder's writes since.
  void routerSaid(RouterUpdate update) {
    _readUnconfirmed();
    switch (update) {
      case RouterUpdate_Path(:final RoutePath? path):
        routed(path);
      case RouterUpdate_Settled(:final PendingWrite pending, :final Settlement settlement):
        _settled(pending, settlement);
      case RouterUpdate_FeedEnded(:final String key, :final Settlement settlement):
        // A server feed's end, for one whose history entry has not come.
        if (_feed case FeedRunning(key: final String running?) when running == key) {
          if (_ports.settledFeed(settlement) case final FeedResult result) _feedEnded(result);
        }
      case RouterUpdate_Account():
        // The home server keeps the account; no feeder's own.
        break;
    }
  }

  /// A feed left pending by an earlier run that the owner never put away: the
  /// sheet asks about it before any new feed.
  void _pendingFromBefore() {
    final PathRouter? router = _ports.router;
    if (router == null) return;
    for (final PendingWrite pending in router.pending()) {
      if (pending.feeder == _state.known.identity && pending.kind == WriteKind.feed && !pending.acknowledged) {
        _uncertain(pending);
        return;
      }
    }
  }

  /// `pending` may have run: shown until the owner puts it away.
  void _uncertain(PendingWrite pending) {
    final FeedState now = _feed;
    if (now case FeedUnanswered(:final String? key) when key != null && key != pending.key) return;
    _setFeed(
      FeedUnanswered(
        portions: pending.portions ?? now.portions,
        sentAt: DateTime.fromMillisecondsSinceEpoch(pending.sentMs),
        key: pending.key,
      ),
    );
  }

  /// What became of `pending`, a feed of this phone's. It settles the feed
  /// the sheet shows only by its key: any other write's settlement, an
  /// earlier feed put away among them, says nothing of it. A feed still
  /// sending has no key yet, so only its own call's answer (`ownCall`)
  /// settles it.
  void _settled(PendingWrite pending, Settlement settlement, {bool ownCall = false}) {
    if (pending.feeder != _state.known.identity || pending.kind != WriteKind.feed) return;
    final FeedState now = _feed;
    final bool ours = switch (now) {
      FeedUnanswered(:final String? key) => key == pending.key,
      FeedRunning(:final String? key) => key == pending.key,
      FeedSending() => ownCall,
      _ => false,
    };
    if (!ours) return;

    final int portions = pending.portions ?? now.portions;
    switch (settlement) {
      case Settlement_Started(:final int feed):
        // Its own call's answer came first: the same feed, already running.
        if (now case FeedRunning(feed: final int running) when running == feed) return;
        // Its end comes on the server's stream or in the history; past the
        // longest such a feed takes, it shows as uncertain, not running.
        _feedStarted(feed, portions, _ports.feedDueSeconds(portions), key: pending.key, path: pending.path);
      case Settlement_Fed(:final int feed):
        final FeedResult? result = _ports.settledFeed(settlement);
        if (result == null) return;
        if (now is FeedRunning && now.feed == feed) {
          _feedEnded(result);
        } else {
          _setFeed(FeedEnded(result: result, sentAt: now.sentAt));
        }
      case Settlement_Refused():
        _setFeed(FeedRefused(refusal: FeedRefusal.other, portions: portions, sentAt: now.sentAt));
      case Settlement_NotSent():
        _setFeed(FeedNotSent(portions: portions, sentAt: now.sentAt));
      case Settlement_Unrecorded():
        // Never known now, and no longer pending: only the bowl can say.
        _setFeed(FeedUnanswered(portions: portions, sentAt: now.sentAt));
      case Settlement_Done() || Settlement_Superseded():
        break;
    }
  }

  static NotSentWhy? _notSentWhy(NotSentReason reason, ServerError? error) => switch ((reason, error?.kind)) {
    (NotSentReason.store, _) => NotSentWhy.notPinned,
    (_, ServerErrorKind.tokenRevoked || ServerErrorKind.accountDisabled) => NotSentWhy.signedOut,
    (_, ServerErrorKind.forbidden) => NotSentWhy.forbidden,
    _ => null,
  };
}
