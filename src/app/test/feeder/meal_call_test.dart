import 'dart:async';

import 'package:flutter/foundation.dart' show Uint8List;
import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/meal_call.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'schedule_history_support.dart';

void main() {
  group('the meal call', () {
    Future<FakeSession> show(WidgetTester tester, CallClip clip, {PickRecording? pick}) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.replies.add(
        FeederReply.call(
          call: CallInfo(writable: true, clip: clip, ms: clip is CallClip_Stored ? 3500 : null),
        ),
      );
      await showTab(tester, MealCallScreen(feeder: feeder, pick: pick ?? () async => null));
      return session;
    }

    testWidgets('uploads what was picked, once', (tester) async {
      final FakeSession session = await show(
        tester,
        const CallClip.none(),
        pick: () async => Uint8List.fromList([1, 2, 3]),
      );
      expect(find.text('No recording stored'), findsOneWidget);
      session.replies
        ..add(planNoRecording)
        ..add(
          const FeederReply.call(call: CallInfo(writable: true, clip: CallClip.stored(bytes: 9, crc: 1), ms: 3500)),
        );

      await tapIn(tester, find.text('Choose a recording'));
      expect(session.requests.last, isA<FeederRequest_UploadCall>());
      expect(find.text('Stored: 3.5 s a play'), findsOneWidget);
    });

    testWidgets('plays the stored clip on the feeder, and says why it did not', (tester) async {
      final FakeSession session = await show(tester, const CallClip.stored(bytes: 9, crc: 1));
      session.requests.clear();
      session.replies
        ..add(const FeederReply.done())
        ..add(const FeederReply.refused(refusal: FeederRefusal.muted()));

      await tapIn(tester, find.text('Play on the feeder'));
      expect(session.requests, [const FeederRequest.playCall()]);
      expect(find.text('Sound is off, or outside its hours.'), findsNothing);

      await tapIn(tester, find.text('Play on the feeder'));
      expect(session.requests, hasLength(2));
      expect(find.text('Sound is off, or outside its hours.'), findsOneWidget);
    });

    testWidgets('offers no play with no clip stored', (tester) async {
      await show(tester, const CallClip.none());
      expect(find.text('Play on the feeder'), findsNothing);
    });

    /// The screen on an open session, holding what `first` answers.
    Future<(FakeFeederPorts, FakeSession)> onScreen(WidgetTester tester, FeederReply first) async {
      final book = FakeBook()..feeders = [kitchen];
      final ports = FakeFeederPorts();
      final feeder = FeederController(book, ports, kitchen);
      addTearDown(feeder.dispose);
      await tester.pump();
      ports.session.opened();
      await tester.pump();
      final FakeSession session = ports.session;
      session.replies.add(first);
      await showTab(tester, MealCallScreen(feeder: feeder, pick: () async => Uint8List.fromList([1])));
      session.requests.clear();
      return (ports, session);
    }

    /// The screen, a recording ready to pick, its session just ended.
    Future<(FakeFeederPorts, FakeSession)> dropped(WidgetTester tester) async {
      final (FakeFeederPorts ports, FakeSession first) = await onScreen(tester, planNoRecording);
      unawaited(first.updates.close());
      await tester.pump();
      return (ports, first);
    }

    const noReach = "Couldn't reach the feeder. Choose the recording again.";

    testWidgets('a recording waits for the session, then goes once', (tester) async {
      final (FakeFeederPorts ports, FakeSession first) = await dropped(tester);

      await tester.tap(find.text('Choose a recording'));
      await settle(tester);
      expect(find.text('Reaching the feeder… keep this screen open.'), findsOneWidget);

      // The controller's first retry comes after 3 s.
      await tester.pump(const Duration(seconds: 3));
      final FakeSession second = ports.session;
      expect(second, isNot(same(first)));
      second.replies
        ..add(planNoRecording)
        ..add(
          const FeederReply.call(call: CallInfo(writable: true, clip: CallClip.stored(bytes: 9, crc: 1), ms: 3500)),
        );
      second.opened();
      await settle(tester);

      expect(first.requests, isEmpty);
      expect(second.requests.first, const FeederRequest.getCall());
      expect(second.requests.whereType<FeederRequest_UploadCall>(), hasLength(1));
      expect(find.text('Stored: 3.5 s a play'), findsOneWidget);
      expect(find.textContaining('Reaching'), findsNothing);
    });

    testWidgets('a dead session loses the read, never the upload', (tester) async {
      final FakeSession session = await show(tester, const CallClip.none(), pick: () async => Uint8List.fromList([1]));
      session.answering = (_) async => const FeederReply.unanswered();
      session.requests.clear();

      await tester.tap(find.text('Choose a recording'));
      await tester.pump(MealCallScreen.reachWait);
      await settle(tester);

      expect(session.requests.whereType<FeederRequest_UploadCall>(), isEmpty);
      expect(find.text(noReach), findsOneWidget);
    });

    testWidgets('no session in time leaves a lasting message, and sends '
        'nothing', (tester) async {
      final (FakeFeederPorts ports, FakeSession first) = await dropped(tester);

      await tester.tap(find.text('Choose a recording'));
      for (var second = 0; second < 70; second++) {
        await tester.pump(const Duration(seconds: 1));
      }

      expect(first.requests, isEmpty);
      expect(ports.session.requests, isEmpty);
      expect(find.text(noReach), findsOneWidget);
      await tester.pump(const Duration(minutes: 1));
      expect(find.text(noReach), findsOneWidget);
      expect(find.text('Choose a recording'), findsOneWidget);
    });

    const stored = FeederReply.call(call: CallInfo(writable: true, clip: CallClip.stored(bytes: 9, crc: 1), ms: 3500));

    testWidgets('a read that never answers counts against the wait', (tester) async {
      final FakeSession session = await show(tester, const CallClip.none(), pick: () async => Uint8List.fromList([1]));
      session.answering = (_) => Completer<FeederReply>().future;
      session.requests.clear();

      await tester.tap(find.text('Choose a recording'));
      await tester.pump(MealCallScreen.reachWait);
      await settle(tester);

      expect(session.requests, hasLength(1), reason: 'the one read that hung');
      expect(find.text(noReach), findsOneWidget);
    });

    /// A write answered `Unanswered` as its session dies with it.
    void dyingOn<T extends FeederRequest>(FakeSession session) {
      session.answering = (request) async {
        if (request is T) {
          unawaited(session.updates.close());
          return const FeederReply.unanswered();
        }
        if (session.updates.isClosed) return const FeederReply.notSent();
        return stored;
      };
    }

    testWidgets('an upload lost with its session drops the old clip and says '
        'only what it knows', (tester) async {
      final (FakeFeederPorts ports, FakeSession first) = await onScreen(tester, stored);
      expect(find.text('Stored: 3.5 s a play'), findsOneWidget);
      dyingOn<FeederRequest_UploadCall>(first);

      await tester.tap(find.text('Replace the recording'));
      await settle(tester);

      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(find.textContaining('Nothing changed'), findsNothing);
      expect(find.text('Stored: 3.5 s a play'), findsNothing);
      expect(find.text('Reading…'), findsOneWidget);

      await tester.pump(const Duration(seconds: 3));
      final FakeSession second = ports.session;
      second.replies.add(planNoRecording);
      second.opened();
      await settle(tester);

      expect(find.text('No recording stored'), findsOneWidget);
      expect(find.textContaining('Nothing changed'), findsNothing);
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(second.requests.whereType<FeederRequest_UploadCall>(), isEmpty);
    });

    testWidgets('an upload lost for good cannot say what the feeder holds', (tester) async {
      final (_, FakeSession first) = await onScreen(tester, stored);
      dyingOn<FeederRequest_UploadCall>(first);

      await tester.tap(find.text('Replace the recording'));
      await tester.pump();
      for (var second = 0; second < 70; second++) {
        await tester.pump(const Duration(seconds: 1));
      }

      expect(find.text("Can't read it right now"), findsOneWidget);
      expect(find.textContaining('Nothing changed'), findsNothing);
      expect(find.textContaining('may or may not have'), findsOneWidget);
    });

    testWidgets('a delete lost with its session says the same', (tester) async {
      final (FakeFeederPorts ports, FakeSession first) = await onScreen(tester, stored);
      dyingOn<FeederRequest_DeleteCall>(first);

      await tester.tap(find.text('Delete the recording'));
      await settle(tester);

      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(find.textContaining('Nothing changed'), findsNothing);
      expect(find.text('Stored: 3.5 s a play'), findsNothing);

      await tester.pump(const Duration(seconds: 3));
      ports.session.replies.add(stored);
      ports.session.opened();
      await settle(tester);
      expect(find.text('Stored: 3.5 s a play'), findsOneWidget);
    });

    testWidgets('two quick taps open one picker', (tester) async {
      var picks = 0;
      final picking = Completer<Uint8List?>();
      await show(
        tester,
        const CallClip.none(),
        pick: () {
          picks++;
          return picking.future;
        },
      );

      await tester.tap(find.text('Choose a recording'));
      await tester.tap(find.text('Choose a recording'));
      await settle(tester);
      expect(picks, 1);

      picking.complete(null);
      await settle(tester);
      final FilledButton button = tester.widget<FilledButton>(find.byType(FilledButton));
      expect(button.onPressed, isNotNull);
    });

    /// A screen opened while an upload an earlier one started is on its way;
    /// what ends the upload, and what the read after it gets.
    Future<void> reopenedUntil(WidgetTester tester, FeederReply end, FeederReply read) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      final uploading = Completer<FeederReply>();
      session.answering = (request) =>
          request is FeederRequest_UploadCall ? uploading.future : Future.value(planNoRecording);
      Widget screen() => MealCallScreen(feeder: feeder, pick: () async => Uint8List.fromList([1]));

      await showTab(tester, screen());
      await tester.tap(find.text('Choose a recording'));
      await settle(tester);
      await tester.pumpWidget(themed(const SizedBox()));
      await showTab(tester, screen());

      session.answering = (_) async => read;
      uploading.complete(end);
      await settle(tester);

      expect(find.text('Reading…'), findsNothing);
      expect(tester.widget<FilledButton>(find.byType(FilledButton)).onPressed, isNotNull);
    }

    testWidgets('a reopened screen says an upload was refused', (tester) async {
      await reopenedUntil(tester, const FeederReply.refused(refusal: FeederRefusal.forbidden()), planNoRecording);

      expect(find.textContaining("isn't allowed"), findsOneWidget);
      expect(find.text('No recording stored'), findsOneWidget);
    });

    testWidgets('a reopened screen says an upload went unanswered', (tester) async {
      await reopenedUntil(tester, const FeederReply.unanswered(), planNoRecording);

      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(find.text('No recording stored'), findsOneWidget);
    });

    testWidgets('a reopened screen reads even after an upload not sent', (tester) async {
      await reopenedUntil(tester, const FeederReply.notSent(), stored);

      expect(find.textContaining('before this went out'), findsOneWidget);
      expect(find.text('Stored: 3.5 s a play'), findsOneWidget);
    });

    testWidgets('a screen opened during an upload does not start another', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      final uploading = Completer<FeederReply>();
      session.answering = (request) =>
          request is FeederRequest_UploadCall ? uploading.future : Future.value(planNoRecording);
      Widget screen() => MealCallScreen(feeder: feeder, pick: () async => Uint8List.fromList([1]));

      await showTab(tester, screen());
      await tester.tap(find.text('Choose a recording'));
      await settle(tester);

      await tester.pumpWidget(themed(const SizedBox()));
      await showTab(tester, screen());
      final FilledButton button = tester.widget<FilledButton>(find.byType(FilledButton));
      expect(button.onPressed, isNull);
      expect(find.text('Working… keep this screen open.'), findsOneWidget);

      session.answering = (_) async => stored;
      uploading.complete(stored);
      await settle(tester);

      expect(find.text('Stored: 3.5 s a play'), findsOneWidget);
      expect(session.requests.whereType<FeederRequest_UploadCall>(), hasLength(1));
      expect(tester.widget<FilledButton>(find.byType(FilledButton)).onPressed, isNotNull);
    });

    testWidgets('a recording refused or unanswered says so, then reads', (tester) async {
      final FakeSession session = await show(tester, const CallClip.none(), pick: () async => Uint8List.fromList([1]));
      session.replies
        ..add(planNoRecording)
        ..add(const FeederReply.refused(refusal: FeederRefusal.forbidden()));
      await tapIn(tester, find.text('Choose a recording'));
      expect(find.textContaining("isn't allowed"), findsOneWidget);

      session.requests.clear();
      session.replies
        ..add(planNoRecording)
        ..add(const FeederReply.unanswered());
      await tapIn(tester, find.text('Choose a recording'));
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(session.requests.whereType<FeederRequest_UploadCall>(), hasLength(1));
      expect(session.requests.last, const FeederRequest.getCall());
    });

    testWidgets('a stored clip can be deleted', (tester) async {
      final FakeSession session = await show(tester, const CallClip.stored(bytes: 9, crc: 1));

      await tapIn(tester, find.text('Delete the recording'));
      expect(session.requests, contains(const FeederRequest.deleteCall()));
    });
  });
}
