import 'dart:async';

import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';

import 'fakes.dart';

/// The path router with no server: each request goes on the last session the
/// app attached for its feeder, as the real one sends it direct. A feed's answer
/// never comes unless a test gives one, so the session's own updates drive the
/// sheet as they do on the direct path.
class FakeRouter with FakeOpaque implements PathRouter {
  final _attached = <String, FakeSession>{};

  /// What happened, in order: `configure` as the app tells it of a kept
  /// server, `disconnect` as it lets one go, `resume` as it hands one over, `attach <feeder>` and `feed <feeder>` here, and `run <feeder>`
  /// as each session starts.
  final log = <String>[];

  /// Every feed asked of it, by feeder.
  final fed = <(String, int)>[];

  /// What a refresh lists, as the server would.
  List<ServerFeeder> listed = const [];
  int refreshes = 0;

  /// The servers it was handed, and how often it let one go.
  final connected = <ServerAccount>[];
  int disconnects = 0;

  // ignore: close_sinks, the test ends with it
  final stream = StreamController<ServerUpdate>.broadcast();

  // ignore: close_sinks, the test ends with it
  final said = StreamController<RouterUpdate>.broadcast();

  /// What the next feed's router answer is; none comes while `null`.
  RoutedFeed? feedAnswer;

  /// What a check of a pending feed finds.
  RetryOutcome retryAnswer = const RetryOutcome.unknown();

  /// The writes it holds pending.
  final held = <PendingWrite>[];
  final acknowledged = <String>[];
  final retried = <String>[];

  /// The way it says each feeder goes; direct when not set.
  final paths = <String, RoutePath?>{};

  @override
  void attach({required FeederSession session}) {
    if (session is! FakeSession) return;
    _attached[session.identity] = session;
    log.add('attach ${session.identity}');
    session.log = log;
  }

  FakeSession? _on(String feeder) {
    final FakeSession? session = _attached[feeder];
    return session == null || session.updates.isClosed ? null : session;
  }

  @override
  Future<RoutedFeed> feed({required String feeder, required int portions}) {
    fed.add((feeder, portions));
    log.add('feed $feeder');
    final FakeSession? session = _on(feeder);
    session?.feed(portions: portions);
    final RoutedFeed? answer = feedAnswer;
    if (answer != null) return Future.value(answer);
    if (session == null) return Future.value(const RoutedFeed.notSent(reason: NotSentReason.noPath));
    return Completer<RoutedFeed>().future;
  }

  @override
  Future<RoutedReply> cancelFeed({required String feeder, required int feed}) async {
    final FakeSession? session = _on(feeder);
    try {
      session?.cancelFeed(feed: feed);
    } on Object {
      return const RoutedReply(path: RoutePath.direct, reply: FeederReply.notSent(), notSent: NotSentReason.session);
    }
    return session == null
        ? const RoutedReply(reply: FeederReply.notSent(), notSent: NotSentReason.noPath)
        : const RoutedReply(path: RoutePath.direct, reply: FeederReply.done());
  }

  @override
  Future<RoutedReply> ask({required String feeder, required FeederRequest request}) async {
    final FakeSession? session = _attached[feeder];
    if (session == null) return const RoutedReply(reply: FeederReply.notSent(), notSent: NotSentReason.noPath);
    return RoutedReply(
      path: RoutePath.direct,
      reply: await session.ask(request: request),
    );
  }

  @override
  RoutePath? path({required String feeder}) => paths.containsKey(feeder) ? paths[feeder] : RoutePath.direct;

  @override
  List<PendingWrite> pending() => List.unmodifiable(held);

  @override
  void acknowledge({required String key}) => acknowledged.add(key);

  /// The record of writes it set aside at its open, until the bowls are checked.
  SetAside? setAsideFile;

  /// Whether a check of the bowls fails to be written down.
  bool bowlsFail = false;

  @override
  SetAside? setAside() => setAsideFile;

  @override
  void bowlsChecked() {
    if (bowlsFail) throw const BridgeError(kind: ErrorKind.store, message: 'disk full');
    setAsideFile = null;
  }

  @override
  Future<RetryOutcome> retry({required String key}) async {
    retried.add(key);
    return retryAnswer;
  }

  @override
  Stream<RouterUpdate> updates() => said.stream;

  @override
  Stream<ServerUpdate> follow({String? lastEventId}) => stream.stream;

  @override
  void connectServer({required ServerSession session, required ServerAccount account}) => connected.add(account);

  @override
  void disconnectServer() {
    disconnects++;
    log.add('disconnect');
  }

  /// The kept accounts each cold start handed over, and how often the phone
  /// said it holds no token for its server.
  final resumed = <ServerAccount?>[];
  int refusals = 0;

  @override
  Future<ServerAccount> resumeServer({required ServerSession session, ServerAccount? cached}) async {
    resumed.add(cached);
    log.add('resume');
    final ServerAccount account = await session.account();
    connected.add(account);
    return account;
  }

  @override
  void refuseServer() => refusals++;

  @override
  Future<Adoption> checkAdoption({required String feeder, required ClientKey key}) =>
      throw UnimplementedError('no server here');

  /// The kept servers the app told it of before anything else.
  final configured = <KnownServer>[];

  @override
  void configureServer({required KnownServer kept}) {
    configured.add(kept);
    log.add('configure');
  }

  /// While set, the next refresh waits for it, and ends as it does.
  Completer<List<ServerFeeder>>? refreshWait;

  @override
  Future<List<ServerFeeder>> refresh() async {
    refreshes++;
    final Completer<List<ServerFeeder>>? wait = refreshWait;
    refreshWait = null;
    return wait == null ? listed : wait.future;
  }

  @override
  Future<void> resolve() async {}

  @override
  Future<Adoption> adopt({required String feeder, required String name, String? address, required ClientKey key}) =>
      throw UnimplementedError('no server here');

  @override
  Future<Adoption> claimPhone({required String feeder, required ClientKey key}) =>
      throw UnimplementedError('no server here');

  @override
  void close() {}
}
