import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/platform/feeder_ports.dart' show FeederPorts, UpdatePush;
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart' show InstallAsk, InstallAsk_Locate;
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/src/rust/api/update.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/state/update/update_state.dart';

/// One feeder's update, for as long as its controller lives. A run goes once:
/// [start] does nothing while one runs or while its end is unknown, wherever
/// the owner has been meanwhile.
class UpdateFlow extends ChangeNotifier {
  /// The update of `_feeder`, reaching outside Dart through `_ports`.
  UpdateFlow(this._feeder, this._ports);

  final FeederController _feeder;
  final FeederPorts _ports;

  /// How the update stands.
  UpdateState get state => _state;
  UpdateState _state = const UpdateIdle();

  /// The build and version this run offered.
  (String, int)? _served;

  UpdatePush? _push;
  StreamSubscription<UpdateProgress>? _run;
  bool _holding = false;
  bool _disposed = false;

  /// Offer the image with this `build` and `version`.
  Future<void> start({required String build, required int version}) async {
    if (_state is! UpdateIdle) return;
    _served = (build, version);
    _set(const UpdateSending());

    final KnownFeeder known = _feeder.state.known;
    _holding = true;
    final bool held = await _ports.holdProcess('Updating ${known.name}');
    if (_disposed) {
      await _ports.releaseProcess();
      return;
    }
    if (_state case final UpdateRun run) _state = run.changed(holdRefused: !held);

    final UpdatePush push = _ports.pushUpdate(known.identity, known.address);
    _push = push;
    _run = push.progress.listen(
      _heard,
      // Rust ends every run with a value; a stream that closes without one
      // lost its thread, and what the feeder did is not known here.
      onDone: () {
        if (_state is UpdateRun) _ended(const UpdateEnd.uncertain());
      },
    );
    _changed();
  }

  void _heard(UpdateProgress progress) {
    final UpdateState now = _state;
    switch (progress) {
      case UpdateProgress_Sending(written: final int sent, length: final int total):
        if (now is UpdateRun) _state = now.changed(written: sent, length: total, reconnecting: false);
      case UpdateProgress_Reconnecting():
        if (now is UpdateRun) _state = now.changed(reconnecting: true);
      case UpdateProgress_Ask(:final int id, :final InstallAsk ask):
        unawaited(_locate(id, ask));
      case UpdateProgress_Activated():
        if (now is UpdateRun) {
          _state = UpdateRestarting(
            written: now.written,
            length: now.length,
            cancelling: now.cancelling,
            holdRefused: now.holdRefused,
          );
        }
      case UpdateProgress_Ended(end: final UpdateEnd last):
        _ended(last);
        return;
    }
    _changed();
  }

  Future<void> _locate(int id, InstallAsk ask) async {
    if (ask is! InstallAsk_Locate) {
      _push?.located(id, null);
      return;
    }
    final String? found = await _ports.locate(ask.identity, _locateWait);
    _push?.located(id, found);
  }

  static const _locateWait = Duration(seconds: 12);

  void _ended(UpdateEnd last) {
    _release();
    _set(UpdateEnded(last));
  }

  void _release() {
    _push = null;
    unawaited(_run?.cancel());
    _run = null;
    if (_holding) {
      _holding = false;
      unawaited(_ports.releaseProcess());
    }
  }

  /// Ask to stop, once. The run reports how it ended.
  Future<FeederReply> cancel() async {
    _cancelling(true);
    final FeederReply reply = await _feeder.ask(const FeederRequest.cancelUpdate());
    _cancelling(false);
    return reply;
  }

  void _cancelling(bool asking) {
    if (_state case final UpdateRun run) _set(run.changed(cancelling: asking));
    if (_state case final UpdateEnded ended) _set(ended.changed(cancelling: asking));
  }

  /// The owner has read an end that says how it went: the firmware screen may
  /// offer the update again, as their own new decision.
  void acknowledge() {
    if (_state case UpdateEnded(endUnknown: false)) _set(const UpdateIdle());
  }

  /// Read what the feeder says of an unknown end. Idle alone settles nothing: a
  /// bank 1 on trial reads idle too, before its verdict. Bank 0 idle means nothing
  /// is under way and the new build is not running, so the owner may decide
  /// again. The served build at or past its version has kept it. Reads go as
  /// often as the owner looks; the end stays until then.
  Future<void> lookAgain() async {
    final UpdateState now = _state;
    if (now is! UpdateEnded || !now.endUnknown) return;
    final FeederReply reply = await _feeder.ask(const FeederRequest.updateState());
    final bool idle = reply is FeederReply_Update && reply.update.phase == UpdatePhase.idle;
    final UpdateInfo? state = reply is FeederReply_Update ? reply.update : null;
    final (String, int)? served = _served;
    final FeederState feeder = _feeder.state;
    final Card? card = feeder.card;

    if (idle && state != null && state.bank == 0) {
      _set(const UpdateIdle());
    } else if (idle &&
        state != null &&
        served != null &&
        card != null &&
        feeder.cardIsCurrent &&
        card.build.toLowerCase() == served.$1.toLowerCase() &&
        state.minVersion >= served.$2) {
      _set(const UpdateEnded(UpdateEnd.confirmed()));
    } else if (_state case final UpdateEnded ended) {
      _set(ended.changed(stillUnknown: true));
    }
  }

  void _set(UpdateState next) {
    _state = next;
    _changed();
  }

  void _changed() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _release();
    super.dispose();
  }
}
