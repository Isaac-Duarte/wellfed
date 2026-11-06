import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/app.dart';
import 'package:wellfed/platform/menu.dart';
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
Future<GoRouter> _window(WidgetTester tester, FeederFleet fleet, FakeBook book, {MenuPorts? menu}) async {
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
    shell: (context, router, pages, child) =>
        DesktopShell(fleet: fleet, router: router, pages: pages, menu: menu, child: child),
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

/// `key` pressed with `modifier` held.
Future<void> _chord(WidgetTester tester, LogicalKeyboardKey modifier, LogicalKeyboardKey key) async {
  await tester.sendKeyDownEvent(modifier);
  await tester.sendKeyEvent(key);
  await tester.sendKeyUpEvent(modifier);
  await settle(tester);
}

Future<void> _ctrl(WidgetTester tester, LogicalKeyboardKey key) => _chord(tester, LogicalKeyboardKey.controlLeft, key);

/// Settings… picked in the Mac's menu bar, as its runner sends it.
Future<void> _menuSettings(WidgetTester tester) async {
  await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
    'wellfed/menu',
    const StandardMethodCodec().encodeMethodCall(const MethodCall('settings')),
    (_) {},
  );
  await settle(tester);
}

/// A test of the desktop build: Linux (or `platform`) for the test body
/// alone, as the binding checks no debug override outlives it.
void _desktopTest(
  String description,
  Future<void> Function(WidgetTester tester) body, {
  TargetPlatform platform = TargetPlatform.linux,
}) => testWidgets(description, (tester) async {
  debugDefaultTargetPlatformOverride = platform;
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

  // Ctrl on Linux, ⌘ on a Mac.
  for (final (TargetPlatform platform, LogicalKeyboardKey command) in [
    (TargetPlatform.linux, LogicalKeyboardKey.controlLeft),
    (TargetPlatform.macOS, LogicalKeyboardKey.metaLeft),
  ]) {
    _desktopTest('shortcuts open settings, adding and the feeders on ${platform.name}', platform: platform, (
      tester,
    ) async {
      final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
      await _window(tester, fleet, book);

      // On a Mac the menu bar's Settings… owns ⌘, (its own test below).
      await _chord(tester, command, LogicalKeyboardKey.comma);
      expect(find.text('APPEARANCE'), platform == TargetPlatform.macOS ? findsNothing : findsOneWidget);

      await _chord(tester, command, LogicalKeyboardKey.keyN);
      expect(find.text('Add page'), findsOneWidget);

      await _chord(tester, command, LogicalKeyboardKey.digit1);
      expect(find.byType(FeederShell), findsOneWidget);

      await _chord(tester, command, LogicalKeyboardKey.digit0);
      expect(find.text('Overview page'), findsOneWidget);
    });
  }

  _desktopTest('on a Mac, Ctrl is not the command key', platform: TargetPlatform.macOS, (tester) async {
    final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
    await _window(tester, fleet, book);

    await _ctrl(tester, LogicalKeyboardKey.comma);
    expect(find.text('APPEARANCE'), findsNothing);
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

  _desktopTest('Settings… in the Mac menu opens the settings page', platform: TargetPlatform.macOS, (tester) async {
    final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
    await _window(tester, fleet, book, menu: const AppMenuPorts());
    await tester.tap(find.text('Kitchen'));
    await settle(tester);

    await _menuSettings(tester);
    expect(find.text('APPEARANCE'), findsOneWidget);
    expect(_selected(tester, 'Settings'), isTrue);
  });

  _desktopTest('a dialog open over the window keeps it from the menu', platform: TargetPlatform.macOS, (tester) async {
    final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
    final GoRouter router = await _window(tester, fleet, book, menu: const AppMenuPorts());
    unawaited(
      showDialog<void>(
        context: router.routerDelegate.navigatorKey.currentContext ?? (throw StateError('no navigator')),
        builder: (_) => const AlertDialog(content: Text('Did it feed?')),
      ),
    );
    await settle(tester);

    await _menuSettings(tester);
    expect(find.text('Did it feed?'), findsOneWidget);
    expect(find.text('Overview page'), findsOneWidget);
  });

  _desktopTest('Settings… picked twice opens one page over Welcome', platform: TargetPlatform.macOS, (tester) async {
    final fleet = FeederFleet(FakeBook(), FakeFeederPorts());
    addTearDown(fleet.dispose);
    await _window(tester, fleet, FakeBook(), menu: const AppMenuPorts());

    await _menuSettings(tester);
    await _menuSettings(tester);
    expect(find.text('APPEARANCE'), findsOneWidget);

    await tester.tap(find.byTooltip('Back'));
    await settle(tester);
    expect(find.text('Overview page'), findsOneWidget);
  });

  for (final (String how, Future<void> Function(WidgetTester) pick) in [
    (
      'the sidebar',
      (tester) async {
        await tester.tap(find.text('Settings').first);
        await settle(tester);
      },
    ),
    ('the menu', _menuSettings),
  ]) {
    _desktopTest('Settings from $how leaves a page pushed over settings', platform: TargetPlatform.macOS, (
      tester,
    ) async {
      final (FeederFleet fleet, FakeBook book) = await _oneFeeder(tester);
      await _window(tester, fleet, book, menu: const AppMenuPorts());
      await _menuSettings(tester);
      // As About and the server pages open: pushed on the navigator, so the
      // router's location still names settings.
      unawaited(
        Navigator.of(tester.element(find.text('APPEARANCE')))
            .push(MaterialPageRoute<void>(builder: (_) => const Scaffold(body: Text('About page')))),
      );
      await settle(tester);
      expect(find.text('About page'), findsOneWidget);

      await pick(tester);
      expect(find.text('About page'), findsNothing);
      expect(find.text('APPEARANCE'), findsOneWidget);
    });
  }
}
