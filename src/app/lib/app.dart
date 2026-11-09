import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/platform/feeder_ports.dart';
import 'package:wellfed/platform/host.dart';
import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/platform/join_ports.dart';
import 'package:wellfed/platform/menu.dart';
import 'package:wellfed/platform/notices.dart';
import 'package:wellfed/platform/pairing_ports.dart';
import 'package:wellfed/platform/phone.dart';
import 'package:wellfed/platform/server_ports.dart';
import 'package:wellfed/screens/alerts/alert_settings_screen.dart';
import 'package:wellfed/screens/alerts/alerts_screen.dart';
import 'package:wellfed/screens/desktop/sidebar_shell.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/screens/home/add_feeder.dart';
import 'package:wellfed/screens/home/home.dart';
import 'package:wellfed/screens/home/set_aside.dart';
import 'package:wellfed/screens/home/welcome.dart';
import 'package:wellfed/screens/install/install_screen.dart';
import 'package:wellfed/screens/server/server_screens.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/screens/setup/join.dart';
import 'package:wellfed/screens/setup/recover_screen.dart';
import 'package:wellfed/screens/setup/setup_screen.dart';
import 'package:wellfed/src/rust/api/server.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/alerts/alert_settings.dart';
import 'package:wellfed/state/alerts/alerts.dart';
import 'package:wellfed/state/fleet/adopt.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/fleet/store_notice.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/state/join/join.dart';
import 'package:wellfed/state/server/add_server.dart';
import 'package:wellfed/state/server/feeder_food.dart';
import 'package:wellfed/state/server/household.dart';
import 'package:wellfed/state/server/pet.dart';
import 'package:wellfed/state/server/server.dart';
import 'package:wellfed/state/server/server_history.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/state/setup/setup.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';

/// The app's routes. `root` is one route whatever the feeders: it follows
/// `book` by itself, because refreshing the router when a feeder is saved
/// would rebuild the stack under a flow still showing its success screen.
///
/// A desktop build passes `shell`, which draws the window around every page:
/// the routes then live in its navigator, so a page opens beside the sidebar.
GoRouter wellfedRouter({
  required Listenable book,
  required WidgetBuilder root,
  required Map<String, WidgetBuilder> pages,
  Widget Function(BuildContext, String identity, int tab)? feeder,
  Widget Function(BuildContext, String identity)? alerts,
  Widget Function(BuildContext, GoRouter router, GlobalKey<NavigatorState> pages, Widget child)? shell,
}) {
  final List<RouteBase> routes = [
    GoRoute(
      path: '/',
      builder: (context, _) => ListenableBuilder(listenable: book, builder: (context, _) => root(context)),
    ),
    for (final MapEntry(key: path, value: page) in pages.entries)
      GoRoute(path: path, builder: (context, _) => page(context)),
    if (feeder != null)
      GoRoute(
        path: '/feeder/:id',
        // go_router keys a page by its route's pattern, so going from one
        // feeder (or tab link) to another would keep the last one's tabs and
        // state. Keying by the location builds each afresh.
        builder: (context, state) => KeyedSubtree(
          key: ValueKey(state.uri.toString()),
          child: feeder(
            context,
            state.pathParameters['id'] ?? '',
            int.tryParse(state.uri.queryParameters['tab'] ?? '') ?? 0,
          ),
        ),
      ),
    if (alerts != null)
      GoRoute(
        path: '/alerts/:id',
        builder: (context, state) =>
            KeyedSubtree(key: ValueKey(state.uri.toString()), child: alerts(context, state.pathParameters['id'] ?? '')),
      ),
  ];
  if (shell == null) return GoRouter(routes: routes);

  final pagesKey = GlobalKey<NavigatorState>();
  late final GoRouter router;
  router = GoRouter(
    routes: [
      ShellRoute(
        navigatorKey: pagesKey,
        builder: (context, _, child) => shell(context, router, pagesKey, child),
        routes: routes,
      ),
    ],
  );
  return router;
}

/// The sidebar's place for the page on top: a page pushed over another
/// names its own location, not the one under it.
Place? placeOf(GoRouter router) {
  final String? path = router.routerDelegate.currentConfiguration.lastOrNull?.matchedLocation;
  if (path == null) return null;
  if (path == '/') return const OverviewPlace();
  if (path == '/add') return const AddPlace();
  if (path == '/settings') return const SettingsPlace();
  if (path.startsWith('/feeder/')) return FeederPlace(Uri.decodeComponent(path.substring('/feeder/'.length)));
  return null;
}

String _pathOf(Place place) => switch (place) {
  OverviewPlace() => '/',
  FeederPlace(:final String identity) => feederPath(identity),
  AddPlace() => '/add',
  SettingsPlace() => '/settings',
};

/// The desktop window around `child`, the page `router` shows in `pages`.
class DesktopShell extends StatefulWidget {
  const DesktopShell({
    super.key,
    required this.fleet,
    required this.router,
    required this.pages,
    required this.child,
    this.menu,
  });

  final FeederFleet fleet;
  final GoRouter router;
  final GlobalKey<NavigatorState> pages;
  final Widget child;

  /// The menu bar, on a Mac.
  final MenuPorts? menu;

  @override
  State<DesktopShell> createState() => _DesktopShellState();
}

class _DesktopShellState extends State<DesktopShell> {
  /// Stops this window's menu listener, and only its own.
  VoidCallback? _stopMenu;

  @override
  void initState() {
    super.initState();
    _stopMenu = widget.menu?.listen(settings: _menuSettings);
  }

  @override
  void dispose() {
    _stopMenu?.call();
    super.dispose();
  }

  /// Settings… from the menu bar, as ⌘, does in the window. A dialog open
  /// over the window keeps it: the dialog asks something first, and the
  /// keyboard shortcut cannot reach past it either.
  void _menuSettings() {
    if (widget.router.routerDelegate.navigatorKey.currentState?.canPop() ?? false) return;
    _go(const SettingsPlace());
  }

  void _go(Place place) => unawaited(_goTo(widget.router, widget.pages, widget.fleet, place));

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.router.routerDelegate,
    builder: (context, _) => SidebarShell(
      fleet: widget.fleet,
      at: placeOf(widget.router),
      go: _go,
      back: () => unawaited(_back(widget.pages)),
      child: widget.child,
    ),
  );
}

/// Leave the pages showing for `place`. Each page above the first is popped
/// through its own PopScope, so a running install or a committed restore
/// asks first, and one that stays keeps the window where it is.
Future<void> _goTo(GoRouter router, GlobalKey<NavigatorState> pages, FeederFleet fleet, Place place) async {
  final NavigatorState? navigator = pages.currentState;
  if (navigator == null) return;
  // Already showing: a second press, or the menu and a shortcut both
  // answering one, must not stack the page again. Only when the router's own
  // page is on top, though: one pushed over it (About, a server page) leaves
  // the router's location as it was, and the press goes back to the place.
  if (_same(placeOf(router), place) && _top(navigator)?.settings is Page<Object?>) return;
  while (navigator.canPop()) {
    final Route<Object?>? before = _top(navigator);
    await navigator.maybePop();
    if (identical(_top(navigator), before)) return;
  }

  final String path = _pathOf(place);
  // With no feeders there is no sidebar to come back by, so the page opens
  // over Welcome with its Back.
  if (fleet.all.isEmpty && place is! OverviewPlace) {
    unawaited(router.push(path));
  } else {
    router.go(path);
  }
}

/// Whether `place` is the page showing. A feeder is never the same: picking
/// it again in the sidebar brings it back to its first tab.
bool _same(Place? showing, Place place) => switch ((showing, place)) {
  (OverviewPlace(), OverviewPlace()) || (AddPlace(), AddPlace()) || (SettingsPlace(), SettingsPlace()) => true,
  _ => false,
};

/// Escape: out of a text field first, then back a page through its PopScope.
Future<void> _back(GlobalKey<NavigatorState> pages) async {
  final FocusNode? focus = FocusManager.instance.primaryFocus;
  // A field's node belongs to the Focus its EditableText builds.
  if (focus?.context?.findAncestorWidgetOfExactType<EditableText>() != null) {
    focus?.unfocus();
    return;
  }
  await pages.currentState?.maybePop();
}

/// The route on top of `navigator`. A predicate that holds at once pops
/// nothing, and is the one way a navigator names its top route.
Route<Object?>? _top(NavigatorState navigator) {
  Route<Object?>? top;
  navigator.popUntil((route) {
    top = route;
    return true;
  });
  return top;
}

class WellfedApp extends StatefulWidget {
  const WellfedApp({super.key, required this.app});

  final AppState app;

  @override
  State<WellfedApp> createState() => _WellfedAppState();
}

class _WellfedAppState extends State<WellfedApp> {
  late final _pairing = AppPairingPorts(widget.app.key);
  static const _phone = AppPhonePorts();

  late final _server = HomeServer(widget.app, const AppServerPorts(), widget.app.router);
  late final _feederPorts = AppFeederPorts(widget.app.key, widget.app.router);
  late final _fleet = FeederFleet(widget.app, _feederPorts, server: _server);
  late final _adopt = AdoptFlow(_feederPorts, _server);

  /// Each server screen's controller, on the session open when it is made.
  late final _serverScreens = ServerScreens(
    server: _server,
    adopt: _adopt,
    newAddServer: () => AddServerFlow(_server),
    newHousehold: () => switch (_server.session) {
      final ServerSession session? => HouseholdFlow(session),
      null => null,
    },
    newPet: (pet) => switch (_server.session) {
      final ServerSession session? => PetFlow(session, pet),
      null => null,
    },
    newFood: (feeder) => switch (_server.session) {
      final ServerSession session? => FeederFoodFlow(session, feeder),
      null => null,
    },
    newHistory: (feeder) => switch (_server.session) {
      final ServerSession session? => ServerHistoryFlow(session, feeder),
      null => null,
    },
  );
  late final AppLifecycleListener _lifecycle;

  /// Alerts at home: Android only, where this phone watches in the
  /// background.
  static final bool _alerts = Platform.isAndroid;
  late final _alertPorts = AppAlertPorts(widget.app.key);
  StreamSubscription<String>? _taps;

  @override
  void initState() {
    super.initState();
    _lifecycle = AppLifecycleListener(onResume: _fleet.resumed);
    if (_alerts) {
      _watched = {for (final KnownFeeder feeder in widget.app.feeders) feeder.identity};
      widget.app.addListener(_feedersChanged);
      // A tap on an alert opens its feeder's alerts.
      _taps = Notices.tapped.listen(_openAlerts);
      unawaited(Notices.launchedFor().then((identity) => identity == null ? null : _openAlerts(identity)));
    }
  }

  /// The feeders this phone knew when the book last changed.
  Set<String> _watched = {};

  /// A feeder this phone forgot takes its alerts with it, and their
  /// notifications come off.
  void _feedersChanged() {
    final Set<String> now = {for (final KnownFeeder feeder in widget.app.feeders) feeder.identity};
    for (final String gone in _watched.difference(now)) {
      unawaited(forgetFeederAlerts(gone));
    }
    _watched = now;
  }

  void _openAlerts(String identity) => unawaited(_router.push('/alerts/$identity'));

  late final GoRouter _router = wellfedRouter(
    // The server's feeders count as the phone's own do: with none at all,
    // the welcome.
    book: Listenable.merge([widget.app, _fleet, widget.app.notice]),
    root: (context) => switch (widget.app.notice.state) {
      final StoreNotice notice? => SetAsideScreen(notice: notice, onRead: () => unawaited(widget.app.notice.read())),
      null when _fleet.all.isEmpty => const WelcomeScreen(),
      null => HomeScreen(
        fleet: _fleet,
        adopt: _adopt,
        pinsProblem: widget.app.pinsProblem,
        askToNotify: _alerts ? () => offerNotifications(widget.app.feeders) : null,
        earlierInstall: () async {
          final ports = AppInstallPorts(widget.app.key);
          return ports.pending(await ports.paths());
        },
      ),
    },
    feeder: (context, identity, tab) => FeederRoute(
      fleet: _fleet,
      identity: identity,
      initialTab: tab,
      server: _serverScreens,
      openAlerts: _alerts ? (context, identity) => unawaited(context.push('/alerts/$identity')) : null,
    ),
    alerts: _alerts
        ? (context, identity) {
            final KnownFeeder? known = widget.app.feeders.where((feeder) => feeder.identity == identity).firstOrNull;
            if (known == null) {
              return const Scaffold(body: Center(child: Text('This phone no longer knows that feeder.')));
            }
            return AlertsScreen(
              name: known.name,
              newController: () => FeederAlertsController(_alertPorts, known),
              openSettings: () => unawaited(context.push('/alert-settings')),
            );
          }
        : null,
    shell: isDesktop
        ? (context, router, pages, child) => DesktopShell(
            fleet: _fleet,
            router: router,
            pages: pages,
            menu: Platform.isMacOS ? const AppMenuPorts() : null,
            child: child,
          )
        : null,
    pages: {
      '/add': (context) => const AddFeederScreen(),
      '/setup': (context) => SetupScreen(newFlow: () => SetupFlow(widget.app, _pairing, _phone)),
      '/settings': (context) => AppSettingsScreen(
        book: widget.app,
        newBackups: () => BackupsFlow(widget.app, AppBackupPorts(widget.app.key)),
        server: _serverScreens,
        alerts: _alerts,
      ),
      if (_alerts)
        '/alert-settings': (context) =>
            AlertSettingsScreen(newController: () => AlertSettingsController(_alertPorts, widget.app)),
      '/join': (context) =>
          JoinScreen(newFlow: () => JoinFlow(widget.app, AppJoinPorts(widget.app.key)), canScan: hostCanScan),
      '/recover': (context) => RecoverScreen(newFlow: () => RecoverFlow(widget.app, _pairing, _phone)),
      '/install': (context) => InstallScreen(newFlow: () => InstallFlow(widget.app, AppInstallPorts(widget.app.key))),
    },
  );

  @override
  void dispose() {
    unawaited(_taps?.cancel());
    if (_alerts) widget.app.removeListener(_feedersChanged);
    _lifecycle.dispose();
    _router.dispose();
    _adopt.dispose();
    _fleet.dispose();
    _server.dispose();
    // A router left open would be taken over by the next one anyway; this
    // ends its updates now.
    widget.app.router?.close();
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
