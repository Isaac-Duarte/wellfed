import 'dart:async';

import 'package:flutter/foundation.dart' show Uint8List;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/meal_call.dart';

import '../support/fakes.dart';
import '../support/harness.dart';

/// The meal call's upload rule, with no screen: a read first, a resend only
/// after "not sent", one upload at a time, and an upload that fails in the
/// bridge left unknown, never stuck and never sent again.
void main() {
  final Uint8List wav = Uint8List.fromList([1, 2, 3]);
  const FeederReply held = FeederReply.call(call: CallInfo(writable: true, clip: CallClip.none()));
  int uploads(FakeSession session) => session.requests.whereType<FeederRequest_UploadCall>().length;

  testWidgets('an upload whose ask throws ends unknown, and the next one goes', (tester) async {
    final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
    session.answering = (request) async {
      if (request is FeederRequest_UploadCall) throw StateError('the bridge panicked');
      return held;
    };

    final FeederReply? reply = await feeder.mealCall.send(wav, wanted: () => true);
    expect(reply, isA<FeederReply_Unanswered>());
    expect(feeder.mealCall.uploading, isNull, reason: 'settled, not stuck');
    expect(uploads(session), 1, reason: 'never sent again');

    session.answering = (request) async => request is FeederRequest_UploadCall ? const FeederReply.done() : held;
    expect(await feeder.mealCall.send(wav, wanted: () => true), isA<FeederReply_Done>());
    expect(uploads(session), 2);
  });

  testWidgets('only "not sent" sends again; an unanswered upload does not', (tester) async {
    final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
    final answers = <FeederReply>[
      const FeederReply.notSent(reason: NotSentReason.session),
      const FeederReply.unanswered(),
    ];
    session.answering = (request) async => request is FeederRequest_UploadCall ? answers.removeAt(0) : held;
    final reads = <FeederReply>[];

    final Future<FeederReply?> sending = feeder.mealCall.send(wav, wanted: () => true, read: reads.add);
    await tester.pump(const Duration(seconds: 1));
    expect(await sending, isA<FeederReply_Unanswered>());
    expect(uploads(session), 2);
    expect(reads, [held, held], reason: 'a read before each upload');
  });

  // "Not sent" can be the same every time (a request the bridge refuses
  // before it goes out) while the read answers: each try waits a beat, and
  // the deadline ends them.
  testWidgets('an upload never sent waits between tries, and stops at the deadline', (tester) async {
    final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
    session.answering = (request) async =>
        request is FeederRequest_UploadCall ? const FeederReply.notSent(reason: NotSentReason.session) : held;

    FeederReply? reply = const FeederReply.done();
    var ended = false;
    unawaited(
      feeder.mealCall.send(wav, wanted: () => true).then((answer) {
        reply = answer;
        ended = true;
      }),
    );
    await tester.pump(const Duration(seconds: 1));
    expect(uploads(session), lessThanOrEqualTo(3), reason: 'a beat between tries');

    await tester.pump(MealCallFlow.reachWait);
    expect(ended, isTrue);
    expect(reply, isNull, reason: 'never sent, so never reached');
    expect(uploads(session), lessThanOrEqualTo(MealCallFlow.reachWait.inMilliseconds ~/ 500 + 1));
  });

  testWidgets('a second send while one uploads gets its answer and sends nothing', (tester) async {
    final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
    final uploaded = Completer<FeederReply>();
    session.answering = (request) => request is FeederRequest_UploadCall ? uploaded.future : Future.value(held);

    final Future<FeederReply?> first = feeder.mealCall.send(wav, wanted: () => true);
    await tester.pump();
    expect(feeder.mealCall.uploading, isNotNull);
    final Future<FeederReply?> second = feeder.mealCall.send(wav, wanted: () => true);
    uploaded.complete(const FeederReply.done());

    expect(await first, isA<FeederReply_Done>());
    expect(await second, isA<FeederReply_Done>());
    expect(uploads(session), 1);
  });

  testWidgets('one asked for by someone gone sends nothing', (tester) async {
    final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
    session.answering = (_) async => held;
    expect(await feeder.mealCall.send(wav, wanted: () => false), isNull);
    expect(uploads(session), 0);
  });
}
