import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import '../support/fakes.dart';
import '../support/harness.dart';

const _settings = SettingsView(
  generation: 5,
  lights: true,
  sound: true,
  volume: 50,
  lightsWindow: Window.between(start: 7 * 60, end: 22 * 60),
  soundWindow: Window.always(),
  desiccantDays: 14,
  mealReminder: MealReminder.before(minutes: 15),
  writable: true,
);

Future<(FeederController, FakeSession)> _settingsTab(WidgetTester tester, {ClientRole role = ClientRole.owner}) async {
  final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
  session.updates.add(SessionUpdate.opened(build: 'test', role: role));
  session.status(fakeCard(settings: _settings));
  await showTab(tester, SettingsTab(feeder: feeder));
  session.requests.clear();
  return (feeder, session);
}

FeederRequest_PutSettings _put(FakeSession session) => session.requests.single as FeederRequest_PutSettings;

/// The settings as the feeder holds them after one change.
SettingsView _withLights(bool lights) => SettingsView(
  generation: 6,
  lights: lights,
  sound: _settings.sound,
  volume: _settings.volume,
  lightsWindow: _settings.lightsWindow,
  soundWindow: _settings.soundWindow,
  desiccantDays: _settings.desiccantDays,
  mealReminder: _settings.mealReminder,
  writable: true,
);

SettingsView _withVolume(int volume) => SettingsView(
  generation: 6,
  lights: _settings.lights,
  sound: _settings.sound,
  volume: volume,
  lightsWindow: _settings.lightsWindow,
  soundWindow: _settings.soundWindow,
  desiccantDays: _settings.desiccantDays,
  mealReminder: _settings.mealReminder,
  writable: true,
);

void main() {
  group('Settings', () {
    testWidgets('a session opening says which build answered', (tester) async {
      final ports = FakeFeederPorts();
      final feeder = FeederController(FakeBook()..feeders = [kitchen], ports, kitchen);
      addTearDown(feeder.dispose);
      await tester.pump();
      ports.session.opened();
      await tester.pump();
      expect(ports.answers, [(kitchen.identity, 'test')]);
    });

    // Offline is the hard case: with no tap on the row or the switch, nothing
    // else keeps them from merging into one node.
    testWidgets('each row is its own node, and each switch its own switch', (tester) async {
      final SemanticsHandle semantics = tester.ensureSemantics();
      final (_, FakeSession session) = await _settingsTab(tester);
      unawaited(session.updates.close());
      await tester.pump();
      await tester.pump(const Duration(seconds: 3));
      await settle(tester);

      final Finder switches = find.byType(Switch);
      expect(switches, findsNWidgets(2));
      final SemanticsNode lights = tester.getSemantics(switches.first);
      final SemanticsNode sounds = tester.getSemantics(switches.last);
      expect(lights.label, isNot(contains('Any time')));
      expect(sounds.label, isNot(contains('7:00 AM')));
      expect(lights, isNot(same(sounds)));
      expect(lights.rect.width, lessThanOrEqualTo(tester.getSize(switches.first).width));

      final SemanticsNode hours = tester.getSemantics(find.text('7:00 AM to 10:00 PM'));
      expect(hours.label, isNot(contains('Any time')));

      semantics.dispose();
    });

    testWidgets('shows what the feeder holds', (tester) async {
      await _settingsTab(tester);

      expect(find.text('7:00 AM to 10:00 PM'), findsOneWidget);
      expect(find.text('Any time'), findsOneWidget);
      expect(find.text('50'), findsOneWidget);
      expect(find.text('15 minutes ahead'), findsOneWidget);
      expect(find.text('Desiccant every 14 days'), findsOneWidget);
      expect(find.text('Next change in 9 days'), findsOneWidget);
      for (final row in ['Wi-Fi', 'Clock', 'Portion size', 'Meal call', 'People', 'Firmware']) {
        expect(find.text(row), findsOneWidget);
      }
      expect(find.text('Diagnostics'), findsOneWidget);
    });

    // A feeder of another build still takes an update from the Firmware
    // row; one of another major takes none from this app.
    for (final (ErrorKind kind, bool offered) in [(ErrorKind.otherBuild, true), (ErrorKind.version, false)]) {
      testWidgets('after a session ended $kind, Firmware is ${offered ? '' : 'not '}offered', (tester) async {
        final (_, FakeSession session) = await _settingsTab(tester);
        session.updates.add(
          SessionUpdate.closed(
            error: BridgeError(kind: kind, message: 'another build'),
          ),
        );
        unawaited(session.updates.close());
        await tester.pump();
        await settle(tester);
        expect(find.text('Firmware'), offered ? findsOneWidget : findsNothing);
        await tester.pump(const Duration(seconds: 3));
      });
    }

    testWidgets('a member sees what a member may change', (tester) async {
      await _settingsTab(tester, role: ClientRole.member);

      expect(find.text('OWNER ONLY'), findsOneWidget);
      expect(find.text('Restart'), findsNothing);
      expect(find.text('Diagnostics'), findsNothing);
      expect(find.text('Firmware updates'), findsOneWidget);
    });

    testWidgets('a toggle goes against the generation it read', (tester) async {
      final (_, FakeSession session) = await _settingsTab(tester);

      await tapIn(tester, find.byType(Switch).first);
      final FeederRequest_PutSettings put = _put(session);
      expect(put.base, 5);
      expect(put.changes.lights, isFalse);
      expect(put.changes.sound, isNull);
    });

    testWidgets('a change refused as stale, or unanswered, says so', (tester) async {
      final (_, FakeSession session) = await _settingsTab(tester);
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.stale(current: 6)))
        ..add(const FeederReply.unanswered());

      await tapIn(tester, find.byType(Switch).first);
      expect(find.textContaining('This changed just now'), findsOneWidget);
      await tapIn(tester, find.byType(Switch).first);
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(session.requests, hasLength(2));
    });

    testWidgets('a switch moves on tap and stays while the change is out', (tester) async {
      final (_, FakeSession session) = await _settingsTab(tester);
      final answer = Completer<FeederReply>();
      session.answering = (_) => answer.future;
      bool lights() => tester.widget<Switch>(find.byType(Switch).first).value;

      await tapIn(tester, find.byType(Switch).first);
      expect(lights(), isFalse);

      // The session puts the feeder's done on the card before the reply.
      session.status(fakeCard(settings: _withLights(false)));
      answer.complete(const FeederReply.done(generation: 6));
      await settle(tester);
      expect(lights(), isFalse);
      expect(find.byType(SnackBar), findsNothing);
    });

    testWidgets('a refused switch moves back and says why', (tester) async {
      final (_, FakeSession session) = await _settingsTab(tester);
      final answer = Completer<FeederReply>();
      session.answering = (_) => answer.future;
      bool lights() => tester.widget<Switch>(find.byType(Switch).first).value;

      await tapIn(tester, find.byType(Switch).first);
      expect(lights(), isFalse);
      answer.complete(const FeederReply.refused(refusal: FeederRefusal.notWritable()));
      await settle(tester);
      expect(lights(), isTrue);
      expect(find.textContaining("can't save changes"), findsOneWidget);
    });

    testWidgets('the volume stays where it was let go until the card shows it', (tester) async {
      final (_, FakeSession session) = await _settingsTab(tester);
      final answer = Completer<FeederReply>();
      session.answering = (_) => answer.future;

      await tester.drag(find.byType(Slider), const Offset(-60, 0));
      await settle(tester);
      final int? asked = _put(session).changes.volume;
      expect(asked, isNotNull);
      expect(asked, lessThan(50));
      expect(find.text('$asked'), findsOneWidget);

      session.status(fakeCard(settings: _withVolume(asked ?? 0)));
      await tester.pump();
      expect(find.text('$asked'), findsOneWidget);
      answer.complete(const FeederReply.done(generation: 6));
      await settle(tester);
      expect(find.text('$asked'), findsOneWidget);
      expect(find.text('50'), findsNothing);
    });

    testWidgets('a refused volume goes back to what the feeder holds', (tester) async {
      final (_, FakeSession session) = await _settingsTab(tester);
      final answer = Completer<FeederReply>();
      session.answering = (_) => answer.future;

      await tester.drag(find.byType(Slider), const Offset(-60, 0));
      await settle(tester);
      expect(find.text('50'), findsNothing);
      answer.complete(const FeederReply.refused(refusal: FeederRefusal.busy()));
      await settle(tester);
      expect(find.text('50'), findsOneWidget);
      expect(find.textContaining("It's busy"), findsOneWidget);
    });

    testWidgets('a window is chosen in its dialog', (tester) async {
      final (_, FakeSession session) = await _settingsTab(tester);

      await tapIn(tester, find.text('Lights'));
      expect(find.text('When can the lights be on?'), findsOneWidget);
      await tapIn(tester, find.text('Any time').last);
      await tapIn(tester, find.widgetWithText(FilledButton, 'Save'));

      expect(_put(session).changes.lightsWindow, const Window.always());
    });

    testWidgets('a muted test sound says why', (tester) async {
      final (_, FakeSession session) = await _settingsTab(tester);
      session.replies.add(const FeederReply.refused(refusal: FeederRefusal.muted()));

      await tapIn(tester, find.byTooltip('Play a test sound'));
      expect(session.requests.single, const FeederRequest.play(tune: Tune.test));
      expect(find.textContaining('Sound is off'), findsOneWidget);
    });

    testWidgets('the feeder rows say where each stands, with Remove at the end and no Restart', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.answering = (request) async => switch (request) {
        FeederRequest_GetCall() => const FeederReply.call(
          call: CallInfo(writable: true, clip: CallClip.stored(bytes: 16800, crc: 1), ms: 4200),
        ),
        _ => const FeederReply.done(),
      };
      session.status(fakeCard(build: '0123456789abcdef', settings: _settings));
      await showTab(tester, SettingsTab(feeder: feeder));

      expect(session.requests, contains(const FeederRequest.getCall()));
      expect(find.text('Your recording · 4 s'), findsOneWidget);
      expect(find.text('0123456789ab'), findsOneWidget);
      expect(find.text('Up 0 min'), findsOneWidget);
      await scrollTo(tester, find.text('Remove from this phone'));
      expect(find.text('Restart'), findsNothing);
      expect(find.text('Remove from this phone'), findsOneWidget);
    });

    testWidgets('removing takes the feeder off this phone once confirmed', (tester) async {
      final book = FakeBook()..feeders = [kitchen];
      final ports = FakeFeederPorts();
      final feeder = FeederController(book, ports, kitchen);
      addTearDown(feeder.dispose);
      await tester.pump();
      ports.session.opened();
      ports.session.status(fakeCard(settings: _settings));
      await showTab(tester, SettingsTab(feeder: feeder));

      await scrollTo(tester, find.text('Remove from this phone'));
      await tapIn(tester, find.text('Remove from this phone'));
      expect(book.feeders, hasLength(1), reason: 'it asks first');
      await tapIn(tester, find.widgetWithText(FilledButton, 'Remove'));
      expect(book.feeders, isEmpty);
    });

    testWidgets('removing asks first, and goes back', (tester) async {
      final (FeederController feeder, FakeSession session) = await _settingsTab(tester);

      await scrollTo(tester, find.text('Remove from this phone'));
      await tapIn(tester, find.text('Remove from this phone'));
      expect(find.textContaining('stays on its People list'), findsOneWidget);
      await tapIn(tester, find.text('Cancel'));
      expect(session.requests, isEmpty);
      expect(feeder.state.known.name, 'Kitchen');
    });
  });
}
