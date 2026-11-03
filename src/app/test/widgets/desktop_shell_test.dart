import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/app.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/theme/theme.dart';

import '../feeder/feeder_support.dart';
import '../support/fakes.dart';

/// The desktop window as the app builds it, around the routes it has. The
/// `/flow` page stands for a running install: it refuses to be left.
Future<GoRouter> _window(WidgetTester tester, FeederFleet fleet, FakeBook book) async {
  tester.view.physicalSize = const Size(1280, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  final GoRouter router = wellfedRouter(
    book: book,
    root: (context) => const Scaffold(body: Text('Overview page')),
    pages: {
      '/add': (context) => const Scaffold(body: Text('Add page')),
      '/form': (context) => const Scaffold(body: TextField()),
      '/flow': (context) => const PopScope(canPop: false, child: Scaffold(body: Text('Running flow'))),
      '/settings': (context) => AppSettingsScreen(book: book, newBackups: () => BackupsFlow(book, FakeBackupPorts())),
    },
    feeder: (context, identity, tab) => FeederRoute(fleet: fleet, identity: identity, initialTab: tab),
    shell: (context, router, pages, child) => DesktopShell(fleet: fleet, router: router, pages: pages, child: child),
  );
  addTearDown(router.dispose);
  await tester.pumpWidget(MaterialApp.router(theme: wellfedTheme(Brightness.light), routerConfig: router));
  await settle(tester);
  return router;
}

/// The fixture's feeder, in a fleet over `book`.
Future<(FeederFleet, FakeBook)> _oneFeeder(WidgetTester tester) => _feeders(tester, [feederKitchen]);

/// `feeders` in a fleet over a book, each session opened.
Future<(FeederFleet, FakeBook)> _feeders(WidgetTester tester, List<KnownFeeder> feeders) async {
  final book = FakeBook()..feeders = feeders;
  final ports = FakeFeederPorts();
  final fleet = FeederFleet(book, ports);
  addTearDown(fleet.dispose);
  await tester.pump();
  for (final FakeSession session in ports.sessions) {
    session.opened();
  }
  await tester.pump();
  return (fleet, book);
}

/// The tab the feeder page shows.
int _tab(WidgetTester tester) => DefaultTabController.of(tester.element(find.byType(TabBar))).index;

/// Whether the sidebar marks `label` as the page showing.
bool _selected(WidgetTester tester, String label) => tester
    .widgetList<Semantics>(find.ancestor(of: find.text(label), matching: find.byType(Semantics)))
    .any((row) => row.properties.selected ?? false);

Future<void> _ctrl(WidgetTester tester, LogicalKeyboardKey key) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(key);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await settle(tester);
}

/// A test of the desktop build: Linux for the test body alone, as the
/// binding checks no debug override outlives it.
void _desktopTest(String description, Future<void> Function(WidgetTester tester) body) =>
    testWidgets(description, (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      try {
        await body(tester);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    });

void main() {
  _desktopTest('the sidebar opens each place and marks the one showing', (tester) async {
    final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
    await _window(tester, fleet, book);
    expect(find.text('Overview page'), findsOneWidget);
    expect(_selected(tester, 'Overview'), isTrue);

    await tester.tap(find.text('Kitchen'));
    await settle(tester);
    expect(find.byType(FeederShell), findsOneWidget);
    expect(_selected(tester, 'Kitchen'), isTrue);
    expect(_selected(tester, 'Overview'), isFalse);
    // Opened from the sidebar, there is no page under it to go back to.
    expect(find.byTooltip('Back'), findsNothing);

    await tester.tap(find.text('Settings').first);
    await settle(tester);
    expect(find.text('APPEARANCE'), findsOneWidget);
    expect(find.byTooltip('Back'), findsNothing);
  });

  _desktopTest('a feeder opened over the overview marks the feeder, and Escape goes back', (tester) async {
    final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
    final GoRouter router = await _window(tester, fleet, book);

    unawaited(router.push('/feeder/$feederIdentity'));
    await settle(tester);
    expect(_selected(tester, 'Kitchen'), isTrue);
    expect(find.byTooltip('Back'), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(tester);
    expect(find.text('Overview page'), findsOneWidget);
    expect(_selected(tester, 'Overview'), isTrue);
  });

  _desktopTest('shortcuts open settings, adding and the feeders', (tester) async {
    final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
    await _window(tester, fleet, book);

    await _ctrl(tester, LogicalKeyboardKey.comma);
    expect(find.text('APPEARANCE'), findsOneWidget);

    await _ctrl(tester, LogicalKeyboardKey.keyN);
    expect(find.text('Add page'), findsOneWidget);

    await _ctrl(tester, LogicalKeyboardKey.digit1);
    expect(find.byType(FeederShell), findsOneWidget);

    await _ctrl(tester, LogicalKeyboardKey.digit0);
    expect(find.text('Overview page'), findsOneWidget);
  });

  _desktopTest('a flow that refuses to be left keeps the window on it', (tester) async {
    final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
    final GoRouter router = await _window(tester, fleet, book);
    unawaited(router.push('/flow'));
    await settle(tester);

    await tester.tap(find.text('Overview'));
    await settle(tester);
    expect(find.text('Running flow'), findsOneWidget);

    await _ctrl(tester, LogicalKeyboardKey.comma);
    expect(find.text('Running flow'), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(tester);
    expect(find.text('Running flow'), findsOneWidget);
  });

  _desktopTest('removing the only feeder goes to the overview', (tester) async {
    final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
    await _window(tester, fleet, book);
    await tester.tap(find.text('Kitchen'));
    await settle(tester);

    await tester.tap(find.widgetWithText(Tab, 'Settings'));
    await settle(tester);
    await feederTap(tester, find.text('Remove from this computer'));
    await tester.tap(find.widgetWithText(FilledButton, 'Remove'));
    await settle(tester);

    expect(find.byType(FeederShell), findsNothing);
    expect(find.text('Overview page'), findsOneWidget);
  });

  _desktopTest('with no feeders, a shortcut opens a page with a way back', (tester) async {
    final fleet = FeederFleet(FakeBook(), FakeFeederPorts());
    addTearDown(fleet.dispose);
    await _window(tester, fleet, FakeBook());

    await _ctrl(tester, LogicalKeyboardKey.comma);
    expect(find.text('APPEARANCE'), findsOneWidget);
    await tester.tap(find.byTooltip('Back'));
    await settle(tester);
    expect(find.text('Overview page'), findsOneWidget);

    await _ctrl(tester, LogicalKeyboardKey.keyN);
    expect(find.text('Add page'), findsOneWidget);
    await _ctrl(tester, LogicalKeyboardKey.digit0);
    expect(find.text('Overview page'), findsOneWidget);
  });

  _desktopTest("another feeder opens on its own Today, not the last one's tab", (tester) async {
    const hall = KnownFeeder(identity: 'bb', name: 'Hall', address: 'lan:4748');
    final (FeederFleet fleet, FakeBook book) = await _feeders(tester, [feederKitchen, hall]);
    await _window(tester, fleet, book);

    await tester.tap(find.text('Kitchen'));
    await settle(tester);
    await tester.tap(find.widgetWithText(Tab, 'History'));
    await settle(tester);
    expect(_tab(tester), historyTab);

    await tester.tap(find.text('Hall'));
    await settle(tester);
    expect(find.text('Hall'), findsNWidgets(2));
    expect(_tab(tester), todayTab);
  });

  _desktopTest('a link to a tab of the feeder showing opens that tab', (tester) async {
    final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
    final GoRouter router = await _window(tester, fleet, book);
    router.go(feederPath(feederIdentity));
    await settle(tester);
    expect(_tab(tester), todayTab);

    router.go(feederPath(feederIdentity, tab: historyTab));
    await settle(tester);
    expect(_tab(tester), historyTab);
  });

  _desktopTest('Escape leaves a text field before it leaves the page', (tester) async {
    final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
    final GoRouter router = await _window(tester, fleet, book);
    unawaited(router.push('/form'));
    await settle(tester);
    await tester.tap(find.byType(TextField));
    await settle(tester);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(tester);
    expect(find.byType(TextField), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(tester);
    expect(find.byType(TextField), findsNothing);
    expect(find.text('Overview page'), findsOneWidget);
  });
}
