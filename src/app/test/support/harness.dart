import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import 'fakes.dart';

const kitchen = KnownFeeder(identity: 'aa', name: 'Kitchen', address: 'lan:4747');

/// A card with everything a screen can read filled in, so a check sees the
/// screen and not its "Reading…" placeholder.
Card fullCard() => fakeCard(
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
