import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/clock.dart';
import 'package:wellfed/screens/feeder/portion.dart';
import 'package:wellfed/src/limits.g.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'feeder_support.dart';

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
    serverDisagrees: false,
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
            serverDisagrees: false,
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
            serverDisagrees: false,
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

    testWidgets('any zone can be chosen by searching for it', (tester) async {
      final FakeSession session = await show(tester);

      await tapIn(tester, find.text('Choose a zone…'));
      expect(find.text('Berlin'), findsOneWidget);
      await tester.enterText(find.byType(TextField), 'new york');
      await tester.pump();
      expect(find.text('Berlin'), findsNothing);
      await tapIn(tester, find.text('New York'));

      final put = session.requests.first as FeederRequest_PutTime;
      expect(put.base, 8);
      expect(put.zone, const ZoneChange.named(name: 'America/New_York'));
      expect(put.ntp, isNull);
      expect(session.requests.last, const FeederRequest.getTime());
    });

    testWidgets('a search that matches nothing says so, and backing out sends nothing', (tester) async {
      final FakeSession session = await show(tester);

      await tapIn(tester, find.text('Choose a zone…'));
      await tester.enterText(find.byType(TextField), 'atlantis');
      await tester.pump();
      expect(find.text('No time zone matches. Try the nearest large city.'), findsOneWidget);
      await tapIn(tester, find.byTooltip('Back'));

      expect(session.requests.whereType<FeederRequest_PutTime>(), isEmpty);
    });

    testWidgets('a zone the feeder keeps can be cleared', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(
        fakeCard(
          time: const TimeInfo(
            serverDisagrees: false,
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
      session.requests.clear();

      await tapIn(tester, find.text('Clear the zone'));
      final put = session.requests.first as FeederRequest_PutTime;
      expect(put.zone, const ZoneChange.clear());
    });

    testWidgets('a feeder that keeps no zone offers no clear', (tester) async {
      await show(tester);

      expect(find.text('None kept'), findsOneWidget);
      expect(find.text('Choose a zone…'), findsOneWidget);
      expect(find.text('Clear the zone'), findsNothing);
    });

    testWidgets('a zone the feeder cannot follow says so', (tester) async {
      final FakeSession session = await show(tester);
      session.replies.add(const FeederReply.notAsked(reason: NotAsked.badZone));

      await tapIn(tester, find.text('Choose a zone…'));
      await tapIn(tester, find.text('Kolkata'));

      expect(find.text("The feeder can't follow that time zone."), findsOneWidget);
    });

    testWidgets('a phone that agrees with the feeder says nothing about it', (tester) async {
      await show(tester);

      expect(find.textContaining('disagree about the time'), findsNothing);
      expect(find.text('Set the time from this phone'), findsNothing);
    });

    testWidgets("an owner's phone that disagrees says so and sets the time only when asked", (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(
        fakeCard(
          time: const TimeInfo(
            serverDisagrees: false,
            generation: 8,
            writable: true,
            ntp: NtpView.off(),
            clockMs: 1000,
            errorMs: 400,
            syncs: 1,
            last: TimeSource.owner,
            phoneAheadMs: 5 * 60000,
          ),
        ),
      );
      await showTab(tester, ClockScreen(feeder: feeder));
      session.requests.clear();

      expect(find.textContaining('This phone and the feeder disagree about the time.'), findsOneWidget);
      expect(find.textContaining('This phone is 5 min ahead of the feeder'), findsOneWidget);
      expect(session.requests, isEmpty);

      await tapIn(tester, find.text('Set the time from this phone'));
      expect(session.requests, [const FeederRequest.syncClock(), const FeederRequest.getTime()]);
    });

    testWidgets('a member whose phone disagrees is told an owner can set it', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(
        fakeCard(
          time: const TimeInfo(
            serverDisagrees: false,
            generation: 8,
            writable: false,
            ntp: NtpView.gateway(port: 123),
            clockMs: 1000,
            errorMs: 400,
            syncs: 1,
            last: TimeSource.ntp,
            phoneAheadMs: -3 * 3600000,
          ),
        ),
      );
      await showTab(tester, ClockScreen(feeder: feeder));

      expect(find.textContaining('This phone is 3 h behind the feeder'), findsOneWidget);
      expect(find.textContaining('An owner can set it'), findsOneWidget);
      expect(find.text('Set the time from this phone'), findsNothing);
    });

    testWidgets('a time server that disagrees with the clock is shown, and an owner may set it', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(
        fakeCard(
          clockSet: false,
          time: const TimeInfo(
            serverDisagrees: true,
            generation: 8,
            writable: true,
            ntp: NtpView.gateway(port: 123),
            syncs: 4,
            last: TimeSource.owner,
          ),
        ),
      );
      await showTab(tester, ClockScreen(feeder: feeder));
      session.requests.clear();

      expect(find.textContaining("The feeder's time server disagrees with its clock."), findsOneWidget);
      expect(session.requests, isEmpty);
      await tapIn(tester, find.text('Set the time from this phone'));
      expect(session.requests, [const FeederRequest.syncClock(), const FeederRequest.getTime()]);
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
      expect(put.ntp, const NtpChoice.off());
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
      session.status(
        fakeCard(
          time: const TimeInfo(serverDisagrees: false, generation: 9, writable: true, ntp: NtpView.off(), syncs: 1),
        ),
      );
      await settle(tester);
      session.requests.clear();

      expect(_chosen(tester, 'Phones only'), isTrue);
      expect(_chosen(tester, 'Your router'), isFalse);
      await tapIn(tester, find.text('Your router'));
      final next = session.requests.first as FeederRequest_PutTime;
      expect(next.base, 9);
    });

    testWidgets('the choices wait for the time read back, so a quick second tap goes nowhere', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      final read = Completer<void>();
      session.answering = (request) async {
        if (request is FeederRequest_GetTime) await read.future;
        return const FeederReply.done();
      };
      session.status(fakeCard(time: time));
      await showTab(tester, ClockScreen(feeder: feeder));
      session.requests.clear();

      await tapIn(tester, find.text('Phones only'));
      await tapIn(tester, find.text('Your router'));
      expect(session.requests.whereType<FeederRequest_PutTime>(), hasLength(1), reason: 'the read is still out');

      read.complete();
      session.status(
        fakeCard(
          time: const TimeInfo(serverDisagrees: false, generation: 9, writable: true, ntp: NtpView.off(), syncs: 1),
        ),
      );
      await settle(tester);
      await tapIn(tester, find.text('Your router'));
      expect(session.requests.whereType<FeederRequest_PutTime>().last.base, 9);
    });

    testWidgets('a public server picked from the list goes as its address', (tester) async {
      final FakeSession session = await show(tester);

      await tapIn(tester, find.text('A time server'));
      await tapIn(tester, find.widgetWithText(ListTile, 'Cloudflare'));

      final put = session.requests.first as FeederRequest_PutTime;
      expect(put.ntp, const NtpChoice.address(address: '162.159.200.1'));
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
    testWidgets('the sheet says grams as Today and the Portion screen do', (tester) async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      session.answering = (request) async => switch (request) {
        FeederRequest_GetCalibration() => const FeederReply.calibration(
          calibration: CalibrationInfo(generation: 1, writable: true, portionMg: 12200, bowl: Bowl.single),
        ),
        _ => const FeederReply.done(),
      };
      await feederShowShell(tester, fleet);
      session.status(fakeCard());
      await settle(tester);
      await feederTap(tester, find.widgetWithText(FilledButton, 'Feed now'));
      await feederTap(tester, find.byTooltip('One more portion'));

      expect(find.text('Feed 2 portions · 24.4 g'), findsOneWidget, reason: 'a tenth kept, not rounded to 24');
    });

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
