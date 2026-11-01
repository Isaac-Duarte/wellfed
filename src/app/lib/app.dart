import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import 'package:wellfed/platform/feeder_ports.dart';
import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/platform/join_ports.dart';
import 'package:wellfed/platform/pairing_ports.dart';
import 'package:wellfed/screens/add_feeder.dart';
import 'package:wellfed/screens/app_settings.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/home.dart';
import 'package:wellfed/screens/install_screen.dart';
import 'package:wellfed/screens/join.dart';
import 'package:wellfed/screens/recover_screen.dart';
import 'package:wellfed/screens/setup_screen.dart';
import 'package:wellfed/screens/welcome.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/app_state.dart';
import 'package:wellfed/state/fleet.dart';
import 'package:wellfed/theme/theme.dart';

/// The app's routes. `root` is one route whatever the feeders: it follows
/// `book` by itself, because refreshing the router when a feeder is saved
/// would rebuild the stack under a flow still showing its success screen.
GoRouter wellfedRouter({
  required Listenable book,
  required WidgetBuilder root,
  required Map<String, WidgetBuilder> pages,
  Widget Function(BuildContext, String identity, int tab)? feeder,
}) => GoRouter(
  routes: [
    GoRoute(
      path: '/',
      builder: (context, _) => ListenableBuilder(listenable: book, builder: (context, _) => root(context)),
    ),
    for (final MapEntry(key: path, value: page) in pages.entries)
      GoRoute(path: path, builder: (context, _) => page(context)),
    if (feeder != null)
      GoRoute(
        path: '/feeder/:id',
        builder: (context, state) => feeder(
          context,
          state.pathParameters['id'] ?? '',
          int.tryParse(state.uri.queryParameters['tab'] ?? '') ?? 0,
        ),
      ),
  ],
);

class WellfedApp extends StatefulWidget {
  const WellfedApp({super.key, required this.app});

  final AppState app;

  @override
  State<WellfedApp> createState() => _WellfedAppState();
}

class _WellfedAppState extends State<WellfedApp> {
  late final _pairing = AppPairingPorts(widget.app.key);

  late final _fleet = FeederFleet(widget.app, AppFeederPorts(widget.app.key));

  late final GoRouter _router = wellfedRouter(
    book: widget.app,
    root: (context) => widget.app.feeders.isEmpty
        ? const WelcomeScreen()
        : HomeScreen(
            fleet: _fleet,
            earlierInstall: () async {
              final ports = AppInstallPorts(widget.app.key);
              return ports.pending(await ports.paths());
            },
          ),
    feeder: (context, identity, tab) => FeederShell.of(_fleet, identity, initialTab: tab),
    pages: {
      '/add': (context) => const AddFeederScreen(),
      '/setup': (context) => SetupScreen(app: widget.app, ports: _pairing),
      '/settings': (context) => AppSettingsScreen(book: widget.app, backups: AppBackupPorts(widget.app.key)),
      '/join': (context) => JoinScreen(book: widget.app, ports: AppJoinPorts(widget.app.key)),
      '/recover': (context) => RecoverScreen(app: widget.app, ports: _pairing),
      '/install': (context) => InstallScreen(book: widget.app, ports: AppInstallPorts(widget.app.key)),
    },
  );

  @override
  void dispose() {
    _router.dispose();
    _fleet.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.app,
    builder: (context, _) => MaterialApp.router(
      title: 'Wellfed',
      debugShowCheckedModeBanner: false,
      theme: wellfedTheme(Brightness.light),
      darkTheme: wellfedTheme(Brightness.dark),
      themeMode: switch (widget.app.appearance) {
        Appearance.system => ThemeMode.system,
        Appearance.light => ThemeMode.light,
        Appearance.dark => ThemeMode.dark,
      },
      routerConfig: _router,
    ),
  );
}
