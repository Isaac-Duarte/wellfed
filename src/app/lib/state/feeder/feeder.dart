import 'dart:async';

import 'package:clock/clock.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/platform/feeder_ports.dart';
import 'package:wellfed/platform/recorder.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart' show ImageCheck;
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/restore/restore.dart';
import 'package:wellfed/state/update/update.dart';
import 'package:wellfed/state/wifi/networks.dart';

part 'feeder_route.dart';
part 'feeder_server.dart';
part 'feeder_session.dart';

/// Where a recording comes from.
typedef PickRecording = Future<Uint8List?> Function();

/// How a change to the address this phone dials ended.
enum AddressChange {
  /// This phone dials it from now on.
  saved,

  /// The text is not `ip` or `ip:port`.
  invalid,

  /// The app's file could not be written, so nothing changed.
  notSaved,
}

/// One known feeder for the Home card: keeps a session open while the app
/// runs, reconnecting with backoff, and sends every request and feed through
/// the path router, which picks the owner's server or that session. Every
/// feed goes through the device API like any other client's; this holds no
/// feeding logic beyond showing what the feeder said.
class FeederController extends ChangeNotifier {
  /// Starts serving `known` at once, and until disposed. Without `ownKey`
  /// the phone holds no key on the feeder, so it opens no session, and the
  /// owner's server is its only way there.
  FeederController(this._book, this._ports, KnownFeeder known, {bool ownKey = true})
    : _state = FeederState(known: known, connection: ownKey ? Connection.looking : Connection.offline),
      _connection = ownKey ? Connection.looking : Connection.offline,
      _ownKey = ownKey {
    _pendingFromBefore();
    if (ownKey) unawaited(_serve());
  }

  static const _locateWait = Duration(seconds: 8);
  static const _retryFirst = Duration(seconds: 3);
  static const _retryMost = Duration(seconds: 15);

  final FeederBook _book;
  final FeederPorts _ports;

  /// This feeder's update: it outlives the firmware screen, so coming back shows
  /// the run as it stands and can never start a second one.
  UpdateFlow get update => _update ??= UpdateFlow(this, _ports);
  UpdateFlow? _update;

  /// The phone holds a key of its own on this feeder, and keeps a session
  /// with it; without one the owner's server is its only way there.
  bool get ownKey => _ownKey;

  /// All that is known of this feeder.
  FeederState get state => _state;
  FeederState _state;

  FeederSession? _session;
  bool _disposed = false;

  /// The phone holds a key of its own on the feeder, and keeps a session.
  final bool _ownKey;

  /// How this phone's own session stands, which [state] shows unless the
  /// owner's server reaches the feeder while it is down.
  Connection _connection;

  /// The feeder as the owner's server last told it, while the server lists it.
  _ServerSide? _server;

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

  /// Hand `text` to Android's share sheet.
  Future<void> share(String text) => _ports.share(text);

  /// Ask the owner for a recording; null when they back out.
  Future<Uint8List?> pickRecording() => _ports.pickRecording();

  /// The phone's microphone, for recording the meal call here.
  Recorder get recorder => _ports.recorder;

  /// The Wi-Fi this phone is on, when Android will say.
  Future<String?> currentNetwork() => _ports.currentNetwork();

  /// The networks this phone hears, for the feeder's Wi-Fi.
  Future<NetworkScan> scanNetworks() => _ports.scanNetworks();

  /// Run `pick`: what it gave, and whether the file could be read.
  Future<(Uint8List?, bool)> readRecording(PickRecording pick) async {
    try {
      return (await pick(), true);
    } on Exception {
      return (null, false);
    }
  }

  /// The image this app carries, for the firmware screen to offer.
  Future<ImageCheck> updateImage() => _ports.updateImage();

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

  /// Read the meal call the feeder holds; [state] keeps it.
  Future<void> loadCall() => ask(const FeederRequest.getCall());

  /// Read the feeder's portion size; [state] keeps it. The answer, when the
  /// feeder gave one, for a screen that shows only what it just read.
  Future<CalibrationInfo?> loadCalibration() async {
    final FeederReply reply = await ask(const FeederRequest.getCalibration());
    return reply is FeederReply_Calibration ? reply.calibration : null;
  }

  /// Take this feeder off this phone.
  Future<void> forgetHere() => _book.forget(_state.known.identity);

  /// Call this feeder `name` on this phone; a blank name changes nothing.
  /// Whether this phone's file holds the name now.
  Future<bool> rename(String name) async {
    final String trimmed = name.trim();
    final KnownFeeder known = _state.known;
    if (trimmed.isEmpty || trimmed == known.name) return true;

    try {
      await _book.rename(known.identity, trimmed);
    } on BridgeError {
      return false;
    }
    _setState(
      _state.copyWith(
        known: KnownFeeder(
          identity: known.identity,
          name: trimmed,
          address: known.address,
          manualAddress: known.manualAddress,
        ),
      ),
    );
    return true;
  }

  /// Dial this feeder at `text` (`ip` or `ip:port`) from now on, or look for it
  /// on the network again when `text` is blank. A session open elsewhere ends
  /// so the next one dials the new address at once.
  Future<AddressChange> setManualAddress(String text) async {
    final String? address = text.trim().isEmpty ? null : _ports.parseAddress(text);
    if (address == null && text.trim().isNotEmpty) return AddressChange.invalid;
    final KnownFeeder known = _state.known;
    if (address == known.manualAddress) return AddressChange.saved;

    try {
      await _book.setManualAddress(known.identity, address);
    } on BridgeError {
      return AddressChange.notSaved;
    }
    _setState(
      _state.copyWith(
        known: KnownFeeder(identity: known.identity, name: known.name, address: known.address, manualAddress: address),
      ),
    );

    _redial = true;
    _session?.close();
    _wake?.call();
    return AddressChange.saved;
  }

  /// The address changed, so the loop dials again without its backoff.
  bool _redial = false;

  /// One request to the feeder, through the path router. A write goes out
  /// once and is never sent again; the reply says how it ended, an unknown
  /// end included. With no router (its pins could not be opened) nothing
  /// goes at all, as a write the router cannot pin must not.
  Future<FeederReply> ask(FeederRequest request) async {
    final PathRouter? router = _ports.router;
    if (router == null) return const FeederReply.notSent();
    final RoutedReply routed = await router.ask(feeder: _state.known.identity, request: request);
    _keep(request, routed.reply);
    return routed.reply;
  }

  /// Keep what a reply says of the meal call and the portion size, whichever
  /// screen asked, so the rows and the feed sheet that show them stay as the
  /// feeder last said. A write to either that ended any other way may have
  /// changed it, so the kept one goes until a read says.
  void _keep(FeederRequest request, FeederReply reply) {
    switch (reply) {
      case FeederReply_Call(:final CallInfo call):
        _setState(_state.copyWith(call: () => call));
      case FeederReply_Calibration(:final CalibrationInfo calibration):
        _setState(_state.copyWith(calibration: () => calibration));
      default:
        switch (request) {
          case FeederRequest_UploadCall() || FeederRequest_DeleteCall():
            _setState(_state.copyWith(call: () => null));
          case FeederRequest_PutCalibration():
            _setState(_state.copyWith(calibration: () => null));
          default:
        }
    }
  }

  Future<void> _serve() async {
    Duration retry = _retryFirst;
    while (!_disposed) {
      final String? address = await _address();
      if (_disposed) return;
      if (address == null) {
        _sessionIs(Connection.offline, error: ErrorKind.connect);
      } else {
        _sessionIs(Connection.connecting);
        final bool opened = await _run(address);
        if (opened) retry = _retryFirst;
      }
      if (_disposed) return;
      if (_redial) {
        _redial = false;
        retry = _retryFirst;
        continue;
      }
      final bool woken = await _pause(retry);
      retry = woken
          ? _retryFirst
          : retry * 2 > _retryMost
          ? _retryMost
          : retry * 2;
    }
  }

  /// Ends the reconnect loop's wait early, if it is waiting.
  void Function()? _wake;

  /// The app is back in the foreground. A session that dropped while it was
  /// away is tried again now, rather than after the backoff its failures in
  /// the background ran up, which left the screen offline for up to 15 s.
  void resumed() => _wake?.call();

  /// Waits `retry`, or until [resumed]; whether it was woken.
  Future<bool> _pause(Duration retry) async {
    final done = Completer<bool>();
    final timer = Timer(retry, () => done.complete(false));
    _wake = () {
      timer.cancel();
      if (!done.isCompleted) done.complete(true);
    };

    final bool woken = await done.future;
    _wake = null;
    return woken;
  }

  /// The owner's address when they gave one, which NSD can't reach past;
  /// otherwise the last address, or NSD's when there is none or it stopped
  /// working.
  Future<String?> _address() async {
    if (_state.known.manualAddress case final String manual?) return manual;
    if (_connection != Connection.offline && _state.known.address != null) {
      return _state.known.address;
    }
    _sessionIs(Connection.looking);
    return await _ports.locate(_state.known.identity, _locateWait) ?? _state.known.address;
  }

  FeedState get _feed => _state.feed;

  void _setFeed(FeedState next) {
    if (next is! FeedRunning) {
      _feedDue?.cancel();
      _feedDue = null;
    }
    _setState(_state.copyWith(feed: next));
  }

  /// The phone's own session is `next` now, having ended on `error`.
  void _sessionIs(Connection next, {ErrorKind? error}) {
    _connection = next;
    _setState(error == null ? _state : _state.copyWith(error: () => error));
  }

  void _setState(FeederState next) {
    _state = _reached(next);
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _wake?.call();
    _feedDue?.cancel();
    _session?.close();
    _update?.dispose();
    super.dispose();
  }
}
