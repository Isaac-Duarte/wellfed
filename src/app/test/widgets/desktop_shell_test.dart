import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/app.dart';
import 'package:wellfed/screens/desktop/sidebar_shell.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/theme/theme.dart';

import '../feeder/feeder_support.dart';
import '../support/fakes.dart';

/// The desktop window as the app builds it, around the routes it has.
Future<GoRouter> _window(WidgetTester tester, FeederFleet fleet) async {
  tester.view.physicalSize = const Size(1280, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  final book = FakeBook();
  final GoRouter router = wellfedRouter(
    book: book,
    root: (context) => const Scaffold(body: Text('Overview page')),
    pages: {
      '/add': (context) => const Scaffold(body: Text('Add page')),
      '/settings': (context) => AppSettingsScreen(book: book, newBackups: () => BackupsFlow(book, FakeBackupPorts())),
    },
    feeder: (context, identity, tab) => FeederRoute(fleet: fleet, identity: identity, initialTab: tab),
    shell: (context, router, pages, child) => ListenableBuilder(
      listenable: router.routerDelegate,
      builder: (context, _) => SidebarShell(
        fleet: fleet,
        at: placeOf(router),
        go: (place) => router.go(switch (place) {
          OverviewPlace() => '/',
          FeederPlace(:final String identity) => '/feeder/$identity',
          AddPlace() => '/add',
          SettingsPlace() => '/settings',
        }),
        back: () => pages.currentState?.maybePop(),
        child: child,
      ),
    ),
  );
  addTearDown(router.dispose);
  await tester.pumpWidget(MaterialApp.router(theme: wellfedTheme(Brightness.light), routerConfig: router));
  await settle(tester);
  return router;
}

/// Whether the sidebar marks `label` as the page showing.
bool _selected(WidgetTester tester, String label) => tester
    .widgetList<Semantics>(find.ancestor(of: find.text(label), matching: find.byType(Semantics)))
    .any((row) => row.properties.selected ?? false);

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
    final (FeederFleet fleet, _) = await feederFleet(tester);
    await _window(tester, fleet);
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
    final (FeederFleet fleet, _) = await feederFleet(tester);
    final GoRouter router = await _window(tester, fleet);

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
    final (FeederFleet fleet, _) = await feederFleet(tester);
    await _window(tester, fleet);

    Future<void> press(LogicalKeyboardKey key) async {
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(key);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await settle(tester);
    }

    await press(LogicalKeyboardKey.comma);
    expect(find.text('APPEARANCE'), findsOneWidget);

    await press(LogicalKeyboardKey.keyN);
    expect(find.text('Add page'), findsOneWidget);

    await press(LogicalKeyboardKey.digit1);
    expect(find.byType(FeederShell), findsOneWidget);

    await press(LogicalKeyboardKey.digit0);
    expect(find.text('Overview page'), findsOneWidget);
  });
}
