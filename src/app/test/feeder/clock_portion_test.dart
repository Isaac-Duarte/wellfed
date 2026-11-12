import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/clock.dart';
import 'package:wellfed/screens/feeder/portion.dart';
import 'package:wellfed/src/limits.g.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import '../support/fakes.dart';
import '../support/harness.dart';

/// Whether the time source named `name` is the ticked one.
bool? _chosen(WidgetTester tester, String name) => tester
    .widget<Semantics>(
      find
          .ancestor(
            of: find.text(name),
            matching: find.byWidgetPredicate((w) => w is Semantics && w.properties.checked != null),
          )
          .first,
    )
    .properties
    .checked;

void main() {
  const time = TimeInfo(
    generation: 8,
    writable: true,
    ntp: NtpView.gateway(port: 123),
    clockMs: 1000,
    errorMs: 400,
    syncs: 1,
    last: TimeSource.ntp,
  );
  const calibration = CalibrationInfo(generation: 2, writable: true, portionMg: 11000, bowl: Bowl.single);

  group('Clock', () {
    Future<FakeSession> show(WidgetTester tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(fakeCard(time: time));
      await showTab(tester, ClockScreen(feeder: feeder));
      session.requests.clear();
      return session;
    }

    testWidgets('shows the time ticking, how it was set, and where it comes from', (tester) async {
      await show(tester);

      expect(find.text('Set by your router · within 1 s'), findsOneWidget);
      expect(find.text('Feeder time right now'), findsOneWidget);
      expect(_chosen(tester, 'Your router'), isTrue);
      String shown() =>
          tester.widget<Text>(find.byWidgetPredicate((w) => w is Text && w.textSpan != null)).textSpan!.toPlainText();
      final String before = shown();
      await tester.pump(const Duration(seconds: 1));
      expect(shown(), isNot(before));
    });

    testWidgets("a feeder on another zone than this phone's offers the phone's", (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(
        fakeCard(
          time: const TimeInfo(
            generation: 8,
            writable: true,
            zone: 'EST5EDT,M3.2.0,M11.1.0',
            zoneNow: ZoneView(abbreviation: 'EST', offsetS: -5 * 3600, seasonal: true),
            phoneZone: 'America/Denver',
            sameAsPhone: false,
            ntp: NtpView.gateway(port: 123),
            syncs: 1,
          ),
        ),
      );
      await showTab(tester, ClockScreen(feeder: feeder));
      session.requests.clear();

      expect(find.text('EST · UTC−5'), findsOneWidget);
      expect(find.text('UTC−5 · follows daylight saving'), findsOneWidget);
      await tapIn(tester, find.text("Use this phone's zone · Denver"));
      final put = session.requests.first as FeederRequest_PutTime;
      expect(put.zone, const ZoneChange.phone());
    });

    testWidgets("the phone's own zone reads as its place", (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(
        fakeCard(
          time: const TimeInfo(
            generation: 8,
            writable: true,
            zone: 'MST7MDT,M3.2.0,M11.1.0',
            zoneNow: ZoneView(abbreviation: 'MST', offsetS: -7 * 3600, seasonal: true),
            phoneZone: 'America/Denver',
            sameAsPhone: true,
            ntp: NtpView.gateway(port: 123),
            syncs: 1,
          ),
        ),
      );
      await showTab(tester, ClockScreen(feeder: feeder));

      expect(find.text('Denver · MST'), findsOneWidget);
      expect(find.text('Same as this phone · follows daylight saving'), findsOneWidget);
      expect(find.textContaining("Use this phone's zone"), findsNothing);
    });

    testWidgets('coming to the screen reads the time again', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(fakeCard(time: time));
      await showTab(tester, ClockScreen(feeder: feeder));

      expect(session.requests, contains(const FeederRequest.getTime()));
    });

    testWidgets('offline it does not pass off an old reading as the time', (tester) async {
      final FakeSession session = await show(tester);
      expect(find.text('Set by your router · within 1 s'), findsOneWidget);

      unawaited(session.updates.close());
      await tester.pump();
      await settle(tester);

      expect(find.text("Can't reach the feeder right now"), findsOneWidget);
      expect(find.textContaining('within'), findsNothing);
      expect(find.text('—'), findsOneWidget);

      // Run out the controller's retry timer before the test ends.
      await tester.pump(const Duration(seconds: 3));
    });

    testWidgets('where the time comes from goes against its generation', (tester) async {
      final FakeSession session = await show(tester);

      await tapIn(tester, find.text('Phones only'));
      final put = session.requests.first as FeederRequest_PutTime;
      expect(put.base, 8);
      expect(put.ntp, 'off');
      expect(put.zone, isNull);
    });

    testWidgets('a time change reads the time again, so the next goes against '
        'the new generation', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(fakeCard(time: time));
      await showTab(tester, ClockScreen(feeder: feeder));
      session.requests.clear();

      await tapIn(tester, find.text('Phones only'));
      expect(session.requests, hasLength(2));
      expect(session.requests.first, isA<FeederRequest_PutTime>());
      expect(session.requests.last, const FeederRequest.getTime());

      // What the session then says of that read.
      session.status(fakeCard(time: const TimeInfo(generation: 9, writable: true, ntp: NtpView.off(), syncs: 1)));
      await settle(tester);
      session.requests.clear();

      expect(_chosen(tester, 'Phones only'), isTrue);
      expect(_chosen(tester, 'Your router'), isFalse);
      await tapIn(tester, find.text('Your router'));
      final next = session.requests.first as FeederRequest_PutTime;
      expect(next.base, 9);
    });

    testWidgets('a public server picked from the list goes as its address', (tester) async {
      final FakeSession session = await show(tester);

      await tapIn(tester, find.text('A time server'));
      await tapIn(tester, find.widgetWithText(ListTile, 'Cloudflare'));

      final put = session.requests.first as FeederRequest_PutTime;
      expect(put.ntp, '162.159.200.1');
    });

    testWidgets('a bad address and an unanswered change say so', (tester) async {
      final FakeSession session = await show(tester);
      session.replies
        ..add(const FeederReply.notAsked(reason: NotAsked.badNtp))
        // The read after that change.
        ..add(const FeederReply.done())
        ..add(const FeederReply.unanswered());

      await tapIn(tester, find.text('A time server'));
      await tester.enterText(find.byType(TextField), 'pool.ntp.org');
      await tapIn(tester, find.widgetWithText(FilledButton, 'Save'));
      expect(find.textContaining('IPv4 address, like'), findsOneWidget);

      await tapIn(tester, find.text('Phones only'));
      expect(find.textContaining('may or may not have'), findsOneWidget);
    });
  });

  group('Portion size', () {
    Future<FakeSession> show(WidgetTester tester, {CalibrationInfo held = calibration}) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(fakeCard(time: time));
      // Every read of the portion answers; anything else takes the queue.
      session.answering = (request) async => switch (request) {
        FeederRequest_GetCalibration() => FeederReply.calibration(calibration: held),
        _ => session.replies.isEmpty ? const FeederReply.done() : session.replies.removeAt(0),
      };
      await showTab(tester, PortionScreen(feeder: feeder));
      session.requests.clear();
      return session;
    }

    testWidgets('shows the portion, and what a meal of two comes to', (tester) async {
      await show(tester);

      expect(find.text('11 g'), findsOneWidget);
      expect(find.text('A meal of 2 portions is about 22 g.'), findsOneWidget);
    });

    testWidgets('a portion steps, or comes from what 10 weighed', (tester) async {
      final FakeSession session = await show(tester);

      await tapIn(tester, find.byTooltip('A gram more'));
      FeederRequest_PutCalibration put = session.requests.whereType<FeederRequest_PutCalibration>().first;
      expect(put.base, 2);
      expect((put.portion as PortionChange_Milligrams).mg, 12000);
      expect(put.bowl, Bowl.single);

      session.requests.clear();
      await tapIn(tester, find.text('Enter what 10 portions weighed'));
      await tester.enterText(find.byType(TextField), '98.4');
      await tapIn(tester, find.widgetWithText(FilledButton, 'Save'));
      put = session.requests.whereType<FeederRequest_PutCalibration>().first;
      final weighed = put.portion as PortionChange_Weighed;
      expect(weighed.grams, '98.4');
      expect(weighed.portions, 10);
    });

    bool enabled(WidgetTester tester, String tooltip) =>
        tester
            .widget<IconButton>(find.ancestor(of: find.byTooltip(tooltip), matching: find.byType(IconButton)))
            .onPressed !=
        null;
    CalibrationInfo weighing(int mg) =>
        CalibrationInfo(generation: 2, writable: true, portionMg: mg, bowl: Bowl.single);

    testWidgets("a portion at the feeder's heaviest steps only down", (tester) async {
      await show(tester, held: weighing(Limits.portionMgMost));
      expect(enabled(tester, 'A gram more'), isFalse);
      expect(enabled(tester, 'A gram less'), isTrue);
    });

    testWidgets("a portion at the feeder's lightest steps only up", (tester) async {
      await show(tester, held: weighing(Limits.portionMgLeast));
      expect(enabled(tester, 'A gram less'), isFalse);
      expect(enabled(tester, 'A gram more'), isTrue);
    });

    testWidgets('a portion refused says so', (tester) async {
      final FakeSession session = await show(tester);
      session.replies.add(const FeederReply.refused(refusal: FeederRefusal.invalid()));

      await tapIn(tester, find.byTooltip('A gram more'));
      expect(find.textContaining('refused that value'), findsOneWidget);
    });
  });
}
