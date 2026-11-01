import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';

const feederIdentity = 'aa';
const feederKitchen = KnownFeeder(identity: feederIdentity, name: 'Kitchen', address: 'lan:4747');

/// One known feeder, its session opened.
Future<(FeederFleet, FakeSession)> feederFleet(WidgetTester tester) async {
  final book = FakeBook()..feeders = [feederKitchen];
  final ports = FakeFeederPorts();
  final fleet = FeederFleet(book, ports);
  addTearDown(fleet.dispose);
  await tester.pump();
  ports.session.opened();
  await tester.pump();
  return (fleet, ports.session);
}

FeederController feederOnly(FeederFleet fleet) => fleet.all.single;

/// `hour`:`minute` today, in milliseconds: a time with no day on it stays bare
/// only while it is today's.
int feederTodayAt(int hour, int minute) {
  final now = DateTime.now();
  return DateTime(now.year, now.month, now.day, hour, minute).millisecondsSinceEpoch;
}

/// A phone-sized frame, so the whole tab fits as the board draws it.
void feederPhone(WidgetTester tester) {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

Future<void> feederShowShell(WidgetTester tester, FeederFleet fleet) async {
  feederPhone(tester);
  await tester.pumpWidget(themed(FeederShell(feeder: feederOnly(fleet))));
  await settle(tester);
}

/// Tap and let the fake's answer come back.
Future<void> feederTap(WidgetTester tester, Finder target) async {
  await tester.ensureVisible(target);
  await tester.pump();
  await tester.tap(target);
  await settle(tester);
}
