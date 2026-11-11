import 'package:flutter/material.dart';

import 'package:wellfed/screens/feeder/history.dart';
import 'package:wellfed/screens/feeder/schedule.dart';
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/screens/feeder/today.dart';
import 'package:wellfed/screens/feeder/unconfirmed.dart';
import 'package:wellfed/screens/server/server_screens.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';

/// One feeder: its name and the tabs (board 7).
///
/// With the owner's server, the name has the way a request goes under it,
/// the food view is one tap away, and History reaches what the server kept.
class FeederShell extends StatelessWidget {
  const FeederShell({super.key, required this.feeder, this.initialTab = 0, this.server, this.openAlerts});

  /// The feeder `identity` of `fleet`, or a plain way back if it is gone: a
  /// link can outlive a feeder this phone has forgotten.
  final FeederController feeder;
  final int initialTab;
  final ServerScreens? server;

  /// Opens this feeder's alerts on this phone, where it watches for them.
  final void Function(BuildContext context, String identity)? openAlerts;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    // A desktop opens a feeder from the sidebar, with nothing under it to go
    // back to, and its tabs sit at the start of the line as a desktop's do.
    final bool back = !isDesktop || Navigator.canPop(context);
    return DefaultTabController(
      length: 4,
      initialIndex: initialTab,
      // A desktop switches tabs at once, as its other apps do; sliding
      // repaints both pages every frame.
      animationDuration: isDesktop ? Duration.zero : null,
      child: Scaffold(
        appBar: AppBar(
          toolbarHeight: isDesktop ? 72 : 64,
          automaticallyImplyLeading: false,
          titleSpacing: back ? 4 : Space.s5,
          leading: back
              ? IconButton(
                  tooltip: 'Back',
                  onPressed: () => Navigator.pop(context),
                  icon: StrokeIcon(Glyph.back, color: c.text),
                )
              : null,
          // The name changes from Settings, so the title follows it.
          title: ListenableBuilder(
            listenable: feeder,
            builder: (context, _) => Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  feeder.state.known.name,
                  style: isDesktop
                      ? TextStyle(fontSize: 24, fontWeight: Fonts.display, letterSpacing: -0.4, color: c.text)
                      : TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
                ),
                // With no server there is one way, and nothing to say.
                if (server != null && feeder.state.serverId != null) PathLine(path: feeder.state.path),
              ],
            ),
          ),
          actions: [
            if (openAlerts case final open?)
              IconButton(
                tooltip: 'Alerts',
                icon: Icon(Icons.notifications_outlined, color: c.text),
                onPressed: () => open(context, feeder.state.known.identity),
              ),
            if (server case final ServerScreens screens?)
              ListenableBuilder(
                listenable: feeder,
                builder: (context, _) => feeder.state.serverId == null
                    ? const SizedBox.shrink()
                    : IconButton(
                        tooltip: 'Food',
                        icon: Icon(Icons.inventory_2_outlined, color: c.text),
                        onPressed: () => screens.openFood(context, feeder),
                      ),
              ),
          ],
          bottom: TabBar(
            labelColor: c.accent,
            unselectedLabelColor: c.textMuted,
            indicatorColor: c.accent,
            indicatorWeight: 3,
            indicatorSize: TabBarIndicatorSize.tab,
            isScrollable: isDesktop,
            tabAlignment: isDesktop ? TabAlignment.start : null,
            padding: isDesktop ? const EdgeInsets.symmetric(horizontal: Space.s3) : null,
            // Four names across a small phone, or in large text, without cutting one off.
            labelPadding: EdgeInsets.symmetric(horizontal: isDesktop ? Space.s4 : Space.s1),
            dividerColor: c.line,
            labelStyle: const TextStyle(fontSize: Sizes.base, fontWeight: Fonts.semibold),
            unselectedLabelStyle: const TextStyle(fontSize: Sizes.base, fontWeight: Fonts.medium),
            tabs: const [
              Tab(text: 'Today'),
              Tab(text: 'Schedule'),
              Tab(text: 'History'),
              Tab(text: 'Settings'),
            ],
          ),
        ),
        body: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            UnconfirmedWrites(feeder: feeder),
            Expanded(
              child: TabBarView(
                // A swipe between tabs is a touch gesture; a mouse drags text.
                physics: isDesktop ? const NeverScrollableScrollPhysics() : null,
                children: [
                  TodayTab(feeder: feeder),
                  ScheduleTab(feeder: feeder),
                  HistoryTab(
                    feeder: feeder,
                    longHistory: switch (server) {
                      final ServerScreens screens? => () => screens.openHistory(context, feeder),
                      null => null,
                    },
                  ),
                  SettingsTab(feeder: feeder),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Gone extends StatelessWidget {
  const _Gone();

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(),
    body: Center(child: Text('This $deviceName no longer knows that feeder.')),
  );
}

/// The feeder a route names, or a screen that says it is gone.
class FeederRoute extends StatelessWidget {
  const FeederRoute({
    super.key,
    required this.fleet,
    required this.identity,
    this.initialTab = 0,
    this.server,
    this.openAlerts,
  });

  final FeederFleet fleet;
  final String identity;
  final int initialTab;
  final ServerScreens? server;
  final void Function(BuildContext context, String identity)? openAlerts;

  /// Follows the fleet, so a feeder forgotten while its page shows gives way
  /// to the plain way back instead of a disposed controller.
  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: fleet,
    builder: (context, _) {
      final FeederController? feeder = fleet[identity];
      return feeder == null
          ? const _Gone()
          : FeederShell(feeder: feeder, initialTab: initialTab, server: server, openAlerts: openAlerts);
    },
  );
}
