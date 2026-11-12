import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/state/clock/clock.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';

/// One feeder's meal-call upload, for as long as its controller lives: the
/// rule for sending a clip, which every screen that sends one shares.
///
/// - A read goes first: a session that died behind the picker can still look
///   open, and a read is free to get lost. The upload then goes on a session
///   seen alive.
/// - Only "not sent" means none of it went out, so only that waits for the
///   session and sends again. An upload that may have started is never sent
///   again.
/// - The wait has a deadline; the upload has none of this flow's, and runs on
///   after the screen that started it goes. A screen opened meanwhile waits for
///   it ([uploading]) rather than start another.
class MealCallFlow {
  /// The upload of `_feeder`'s clips.
  MealCallFlow(this._feeder);

  final FeederController _feeder;

  /// How long a send waits for the session to come back, in all: the picker
  /// can sit open long enough for it to end behind it. A reconnect can take
  /// about 38 s: the longest retry (15 s), a look on the network (8 s), a
  /// connect (5 s) and the feeder's welcome (10 s).
  static const reachWait = Duration(seconds: 60);

  /// The upload on its way, until it ends however it ends.
  Future<FeederReply>? get uploading => _uploading;
  Future<FeederReply>? _uploading;

  /// Send `wav` once the session answers; `null` if it does not in time, or
  /// once `wanted` says whoever asked has gone. `read` hears what the read
  /// that proved the session alive says the feeder holds, and `reaching` that
  /// the session is being waited for again. An upload already on its way is
  /// the answer, and nothing more is sent.
  Future<FeederReply?> send(
    Uint8List wav, {
    required bool Function() wanted,
    void Function(FeederReply read)? read,
    void Function()? reaching,
  }) async {
    if (_uploading case final Future<FeederReply> inFlight) return inFlight;
    final deadline = Deadline(reachWait);
    try {
      while (true) {
        final FeederReply? answered = await answeredRead(deadline, wanted);
        if (answered == null || !wanted()) return null;
        read?.call(answered);

        final FeederReply reply = await _upload(wav);
        if (reply is! FeederReply_NotSent) return reply;
        if (!wanted()) return null;
        reaching?.call();
        // A request refused before any of it went out can be refused so on
        // every try, while its read answers: a beat between tries keeps that
        // from spinning until the deadline.
        await deadline.beat();
        if (!wanted()) return null;
      }
    } finally {
      deadline.cancel();
    }
  }

  /// The first read the feeder answers, waiting out a lost session; `null` once
  /// `deadline` passes or `wanted` says whoever asked has gone.
  Future<FeederReply?> answeredRead(Deadline deadline, bool Function() wanted) async {
    while (!deadline.over) {
      if (_feeder.state.connection == Connection.online) {
        final FeederReply? read = await Future.any<FeederReply?>([
          _feeder.ask(const FeederRequest.getCall()),
          deadline.done.then((_) => null),
        ]);
        if (read == null || !wanted()) return null;
        if (read is! FeederReply_NotSent && read is! FeederReply_Unanswered) {
          return read;
        }
      }
      await deadline.beat();
      if (!wanted()) return null;
    }
    return null;
  }

  /// One upload, marked on its way until it ends. One whose ask fails in the
  /// bridge may have sent some of the clip: unknown, as an unanswered one is,
  /// and the mark is cleared all the same.
  Future<FeederReply> _upload(Uint8List wav) async {
    final Future<FeederReply> sending = _ask(FeederRequest.uploadCall(wav: wav));
    _uploading = sending;
    try {
      return await sending;
    } finally {
      _uploading = null;
    }
  }

  Future<FeederReply> _ask(FeederRequest request) async {
    try {
      return await _feeder.ask(request);
    } on Object {
      return const FeederReply.unanswered();
    }
  }
}
