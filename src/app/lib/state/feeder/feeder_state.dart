import 'package:flutter/foundation.dart';

import 'package:wellfed/src/roles.g.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';

/// How this phone reaches a feeder: its own session, or the owner's server
/// while that session is down and the server holds the feeder.
enum Connection {
  /// Looking for the feeder's address on the LAN.
  looking,

  /// Opening a session to an address.
  connecting,

  /// The session is open, or the server's link to the feeder is.
  online,

  /// Not reachable; [FeederState.error] says how the last try ended.
  offline,

  /// Reachable, but it serves another major of the protocol
  /// ([ErrorKind.version]): not even an update goes to it from this app,
  /// which the owner replaces with a version that speaks it.
  otherVersion,
}

/// Why a running feed went lost, which decides what the sheet may claim.
enum FeedLoss {
  /// The session ended while the feed ran.
  link,

  /// The session is up, and no end has come past the longest a feed can take.
  overdue,
}

/// One feed from this phone, from its confirmation to what became of it.
sealed class FeedState {
  /// A feed of [portions], sent at [sentAt].
  const FeedState({this.portions = 0, this.sentAt});

  /// What the owner asked for, or the feeder reports.
  final int portions;

  /// When this phone sent the feed, for "We sent the feed at …".
  final DateTime? sentAt;

  /// The feed is under way, so no other may start.
  bool get active => false;
}

/// No feed to show.
final class FeedIdle extends FeedState {
  /// The state with no feed.
  const FeedIdle();
}

/// Sent; waiting for the feeder to reserve it.
final class FeedSending extends FeedState {
  /// A feed that went out at [sentAt].
  const FeedSending({required super.portions, required super.sentAt});

  @override
  bool get active => true;
}

/// The feeder runs it as [feed].
final class FeedRunning extends FeedState {
  /// A feed the feeder reserved.
  const FeedRunning({
    required this.feed,
    required super.portions,
    required super.sentAt,
    this.cancelAsked = false,
    this.cancelUnconfirmed = false,
    this.key,
    this.path,
  });

  /// The feeder's number for it.
  final int feed;

  /// Its idempotency key, once the router has answered for it.
  final String? key;

  /// The way it went, once the router has answered for it: through the
  /// server, its end comes on the server's stream, and the phone's own
  /// session closing says nothing of it.
  final RoutePath? path;

  /// "Stop feeding" went out and nothing has come of it yet. Only the feed's
  /// end says it stopped.
  final bool cancelAsked;

  /// "Stop feeding" was refused or went unanswered; the feed may still run
  /// to its end.
  final bool cancelUnconfirmed;

  @override
  bool get active => true;
}

/// It ended, and [result] says how.
final class FeedEnded extends FeedState {
  /// A feed with a known end.
  const FeedEnded({required this.result, required super.sentAt, this.endedAt}) : super(portions: 0);

  /// What the feeder recorded, which may itself be uncertain.
  final FeedResult result;

  /// When its end arrived, if that can be trusted.
  final DateTime? endedAt;

  @override
  int get portions => result.requested;
}

/// The feeder said no, so nothing was fed.
final class FeedRefused extends FeedState {
  /// A feed the feeder turned down.
  const FeedRefused({required this.refusal, required super.portions, required super.sentAt});

  /// Why.
  final FeedRefusal refusal;
}

/// No answer came: the feed may have run. It is never sent again, and no later
/// feed's end is taken for it: the bowl or the history says. A new feed waits
/// until the owner puts it away.
final class FeedUnanswered extends FeedState {
  /// A feed that went out and heard nothing back.
  const FeedUnanswered({required super.portions, required super.sentAt, this.key});

  /// Its idempotency key while the router keeps it pending: a check asks its
  /// own path what became of it, and putting it away lets a new feed go.
  final String? key;
}

/// The request never went out: nothing was fed.
final class FeedNotSent extends FeedState {
  /// A feed that never left this phone.
  const FeedNotSent({required super.portions, required super.sentAt, this.why});

  /// Why, when the router said more than that the feeder was out of reach.
  final NotSentWhy? why;
}

/// Why a feed never went out, as the feed sheet tells it.
enum NotSentWhy {
  /// The server refused this phone's token, or the account stopped: log in again.
  signedOut,

  /// This account may not feed.
  forbidden,

  /// The phone could not write the pending feed down first, so it held it back.
  notPinned,

  /// The record of writes that may have run was set aside, and the owner
  /// has not yet said the bowls are checked.
  setAside,
}

/// The link went while the feed ran, or its end is overdue. If the feeder
/// stayed on it finishes on its own and its history keeps the result, which
/// replaces this when it arrives. A power cut may leave only the bowl to say.
final class FeedAdrift extends FeedState {
  /// A running feed whose end this phone may not hear.
  const FeedAdrift({required this.feed, required this.by, required super.portions, required super.sentAt});

  /// The feeder's number for it.
  final int feed;

  /// Why it went lost.
  final FeedLoss by;
}

/// All that is known of one feeder.
@immutable
final class FeederState {
  /// The state of a feeder not yet looked for.
  const FeederState({
    required this.known,
    this.connection = Connection.looking,
    this.error,
    this.card,
    this.cardAt,
    this.cardFresh = false,
    this.role,
    this.feed = const FeedIdle(),
    this.history = const [],
    this.historyRead = false,
    this.clients = const [],
    this.call,
    this.calibration,
    this.path,
    this.serverId,
    this.viewer = false,
    this.unconfirmed = const [],
    this.skew,
    this.serverSkew,
  });

  /// What this phone keeps about the feeder.
  final KnownFeeder known;

  /// How the session stands.
  final Connection connection;

  /// How the last try to reach it ended, if it failed.
  final ErrorKind? error;

  /// The last status the feeder sent.
  final Card? card;

  /// When [card] was read, for an offline card's age.
  final DateTime? cardAt;

  /// [card] was read in the session now open, not an earlier one.
  final bool cardFresh;

  /// What this phone may do on the feeder.
  final ClientRole? role;

  /// This phone's feed.
  final FeedState feed;

  /// The feeder's history as this session has read it, oldest first.
  final List<HistoryEntry> history;

  /// [history] holds everything the feeder kept when this session opened.
  final bool historyRead;

  /// The clients the feeder lists, once asked for.
  final List<ClientView> clients;

  /// The meal call the feeder last said it holds, for the settings row.
  final CallInfo? call;

  /// The portion size the feeder last said, for the feed sheet's grams.
  final CalibrationInfo? calibration;

  /// The way a new request goes now, as the router last said: through the
  /// owner's server or this phone's own session; `null` when it reaches the
  /// feeder no way at all, or before the router has said.
  final RoutePath? path;

  /// The owner's server's id for it, while the server lists it.
  final String? serverId;

  /// The account the server knows this phone by may only look: it feeds
  /// through neither the server nor the phone's own key, so Feed is hidden.
  final bool viewer;

  /// Changes other than a feed that may or may not have reached the feeder,
  /// held on the path they went until a check or the owner puts them away.
  final List<PendingWrite> unconfirmed;

  /// How the feeder's API revision stands against this app's, from the
  /// last session's welcome: which side to update, never a reason to offer
  /// the feeder an older build. `null` before a session has opened.
  final Skew? skew;

  /// The same against the owner's server, as the server last said.
  final Skew? serverSkew;

  /// The card is what this session read, not an earlier one.
  bool get cardIsCurrent => connection == Connection.online && cardFresh;

  /// Whether the feeder would take a feed from this phone now: its role may
  /// feed by the feeder's own matrix, where a role a newer build added grants
  /// nothing. A role no session has named yet is left to the feeder.
  bool get canFeed =>
      cardIsCurrent &&
      !feed.active &&
      !viewer &&
      switch (role) {
        final ClientRole role => mayAsk(role, Ask.feed),
        null => true,
      } &&
      (card?.ready ?? false);

  /// This state with the named parts changed. A nullable part takes a function,
  /// so that null can be set.
  FeederState copyWith({
    KnownFeeder? known,
    Connection? connection,
    ErrorKind? Function()? error,
    Card? card,
    DateTime? cardAt,
    bool? cardFresh,
    ClientRole? role,
    FeedState? feed,
    List<HistoryEntry>? history,
    bool? historyRead,
    List<ClientView>? clients,
    CallInfo? Function()? call,
    CalibrationInfo? Function()? calibration,
    RoutePath? Function()? path,
    String? Function()? serverId,
    bool? viewer,
    List<PendingWrite>? unconfirmed,
    Skew? Function()? skew,
    Skew? Function()? serverSkew,
  }) => FeederState(
    known: known ?? this.known,
    connection: connection ?? this.connection,
    error: error == null ? this.error : error(),
    card: card ?? this.card,
    cardAt: cardAt ?? this.cardAt,
    cardFresh: cardFresh ?? this.cardFresh,
    role: role ?? this.role,
    feed: feed ?? this.feed,
    history: history == null ? this.history : List.unmodifiable(history),
    historyRead: historyRead ?? this.historyRead,
    clients: clients == null ? this.clients : List.unmodifiable(clients),
    call: call == null ? this.call : call(),
    calibration: calibration == null ? this.calibration : calibration(),
    path: path == null ? this.path : path(),
    serverId: serverId == null ? this.serverId : serverId(),
    viewer: viewer ?? this.viewer,
    unconfirmed: unconfirmed == null ? this.unconfirmed : List.unmodifiable(unconfirmed),
    skew: skew == null ? this.skew : skew(),
    serverSkew: serverSkew == null ? this.serverSkew : serverSkew(),
  );
}
