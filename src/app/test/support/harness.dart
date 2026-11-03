import 'dart:typed_data';

import 'package:clock/clock.dart';
import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import 'fakes.dart';

const kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');

/// The moment the fixture stands at: 3 PM on Monday 2 November 2026, on the
/// host's own clock, so its times read the same in any zone.
final DateTime fixtureNow = DateTime(2026, 11, 2, 15);

/// `hour` o'clock on `day` November 2026, in milliseconds.
int _november(int day, int hour) => DateTime(2026, 11, day, hour).millisecondsSinceEpoch;

/// `body` with the app's clock stopped at [fixtureNow], so "in 17 h" and
/// "Yesterday" don't drift with the day a golden runs.
Future<T> atFixtureNow<T>(Future<T> Function() body) => withClock(Clock.fixed(fixtureNow), body);

/// This morning's planned meal, delivered in full.
final FeedResult fixtureFed = fakeFed(feed: 41, atMs: _november(2, 8), by: const FedBy.plan(slot: 0));

/// A card with everything a screen can read filled in, so a check sees the
/// screen and not its "Reading…" placeholder: fed at 8 this morning, the next
/// meal at 8 tomorrow, the clock set by the router at 6.
Card fullCard() => fakeCard(
  build: '3f2a9c1d7e40b5a6',
  uptimeMs: 3 * 24 * 3600 * 1000,
  nextMeal: NextMealView(slot: 0, atMs: _november(3, 8), minute: 8 * 60, portions: 2),
  lastFed: fixtureFed,
  meals: [
    MealView(
      slot: 0,
      minute: 8 * 60,
      portions: 2,
      call: 0,
      standing: MealStanding.fed(result: fixtureFed),
    ),
  ],
  time: TimeInfo(
    generation: 1,
    writable: true,
    zone: 'MST7MDT,M3.2.0,M11.1.0',
    ntp: const NtpView.gateway(port: 123),
    syncs: 4,
    last: TimeSource.ntp,
    lastAtMs: _november(2, 6),
  ),
  plans: PlansView(
    generation: 3,
    writable: true,
    plans: [
      const PlanView(minute: 8 * 60, portions: 2, days: Weekday.values, enabled: true, call: 0),
      const PlanView(minute: 18 * 60, portions: 1, days: Weekday.values, enabled: false, call: 2),
      ...List<PlanView?>.filled(8, null),
    ],
  ),
  settings: const SettingsView(
    generation: 5,
    lights: true,
    sound: true,
    volume: 50,
    lightsWindow: Window.between(start: 7 * 60, end: 22 * 60),
    soundWindow: Window.always(),
    desiccantDays: 14,
    mealReminder: MealReminder.before(minutes: 15),
    writable: true,
  ),
);

/// The rest of what the full feeder says when asked: two phones, a
/// calibrated 12 g portion and a recorded 4 s meal call.
void answerAsFull(FakeSession session) => session.answering = (request) async => switch (request) {
  FeederRequest_ListClients() => FeederReply.clients(
    clients: [
      ClientView(key: Uint8List.fromList([1]), fingerprint: '01', label: 'Pixel', role: ClientRole.owner, you: true),
      ClientView(
        key: Uint8List.fromList([2]),
        fingerprint: '02',
        label: "Sam's phone",
        role: ClientRole.member,
        you: false,
      ),
    ],
  ),
  FeederRequest_GetCalibration() => const FeederReply.calibration(
    calibration: CalibrationInfo(
      generation: 2,
      writable: true,
      portionMg: 12000,
      bowl: Bowl.single,
      phase: Phase.atMark,
    ),
  ),
  FeederRequest_GetCall() => const FeederReply.call(
    call: CallInfo(writable: true, clip: CallClip.stored(bytes: 64000, crc: 1), ms: 4000),
  ),
  _ => const FeederReply.done(),
};

/// One feeder's controller, its session opened.
Future<(FeederController, FakeSession)> oneFeeder(WidgetTester tester) async {
  final book = FakeBook()..feeders = [kitchen];
  final ports = FakeFeederPorts();
  final feeder = FeederController(book, ports, kitchen);
  addTearDown(feeder.dispose);
  await tester.pump();
  ports.session.opened();
  await tester.pump();
  return (feeder, ports.session);
}

/// `tab` on a phone-sized frame, as the boards draw it.
Future<void> showTab(WidgetTester tester, Widget tab) async {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(themed(Scaffold(body: tab)));
  await settle(tester);
}

/// Tap and let the fake's answer come back.
Future<void> tapIn(WidgetTester tester, Finder target) async {
  await tester.ensureVisible(target);
  await tester.pump();
  await tester.tap(target);
  await settle(tester);
}
