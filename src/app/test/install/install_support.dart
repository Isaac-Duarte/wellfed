import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/install/install_screen.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/theme/theme.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';

/// The screen under a router, as the app has it, so leaving works.
Widget installApp(FakeBook book, FakeInstallPorts ports, {Brightness? brightness}) {
  final router = GoRouter(
    initialLocation: '/install',
    routes: [
      GoRoute(path: '/', builder: (_, _) => const Text('Home')),
      GoRoute(
        path: '/install',
        builder: (_, _) => InstallScreen(newFlow: () => InstallFlow(book, ports)),
      ),
    ],
  );
  return MaterialApp.router(
    theme: wellfedTheme(brightness ?? Brightness.light),
    debugShowCheckedModeBanner: false,
    routerConfig: router,
  );
}

/// The install's first screen.
Future<void> installOpen(WidgetTester tester, FakeInstallPorts ports, {FakeBook? book}) async {
  // Tall enough that a whole screen is built, not just what scrolls into view.
  tester.view.physicalSize = const Size(1080, 6000);
  tester.view.devicePixelRatio = 3;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(installApp(book ?? FakeBook(), ports));
  await settle(tester);
}

Future<void> installTap(WidgetTester tester, String text) async {
  await tester.ensureVisible(find.text(text));
  await settle(tester);
  await tester.tap(find.text(text));
  await settle(tester);
}

/// From the first screen to a run through the Reset hold, with the reset
/// route's broker running.
Future<FakeRun> installToRun(WidgetTester tester, FakeInstallPorts ports, {FakeBook? book}) async {
  await installOpen(tester, ports, book: book);
  await installTap(tester, 'Continue');
  await tester.enterText(find.byType(TextField).at(0), 'Maple St');
  await tester.enterText(find.byType(TextField).at(1), 'correcthorse');
  await tester.pump();
  await installTap(tester, 'Continue');
  await installHoldReset(tester);
  return ports.run;
}

/// Through the Reset screen: what it erases, and the hold.
Future<void> installHoldReset(WidgetTester tester) async {
  await installTap(tester, 'I understand its meals will be erased');
  await installTap(tester, 'I held Reset');
}

/// A run that reached the given screen by the events a real one sends.
Future<FakeRun> installAtConfirm(WidgetTester tester, FakeInstallPorts ports) async {
  final FakeRun run = await installToRun(tester, ports);
  run.ask(9, const InstallAsk.recognizeFeeder(device: 'AF_TEST', address: '192.168.1.40'));
  await settle(tester);
  return run;
}

Future<FakeRun> installAtClaim(WidgetTester tester, FakeInstallPorts ports) async {
  final FakeRun run = await installToRun(tester, ports);
  run.event(const InstallEvent.setupStarting());
  await settle(tester);
  return run;
}

Future<FakeRun> installAtFinishing(WidgetTester tester, FakeInstallPorts ports) async {
  final FakeRun run = await installToRun(tester, ports);
  run.event(InstallEvent.paired(identity: Uint8List.fromList(const [0xab, 0xcd]), rebooting: true));
  await settle(tester);
  return run;
}

Future<FakeRun> installStopped(
  WidgetTester tester,
  FakeInstallPorts ports,
  StopCase stop, {
  Resume resume = Resume.nothing,
  String detail = 'its words',
}) async {
  final FakeRun run = await installToRun(tester, ports);
  run.stopped(stop, resume: resume, detail: detail);
  await settle(tester);
  return run;
}
