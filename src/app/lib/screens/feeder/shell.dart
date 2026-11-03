import 'dart:async';

import 'package:flutter/material.dart';

import 'package:wellfed/screens/feeder/history.dart';
import 'package:wellfed/screens/feeder/schedule.dart';
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/screens/feeder/today.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/ask_text.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/replies.dart';

/// One feeder: its name, the menu, and the tabs (board 7).
class FeederShell extends StatelessWidget {
  const FeederShell({super.key, required this.feeder, this.initialTab = 0});

  /// The feeder `identity` of `fleet`, or a plain way back if it is gone: a
  /// link can outlive a feeder this phone has forgotten.
  final FeederController feeder;
  final int initialTab;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return DefaultTabController(
      length: 4,
      initialIndex: initialTab,
      child: Scaffold(
        appBar: AppBar(
          toolbarHeight: 64,
          automaticallyImplyLeading: false,
          titleSpacing: 4,
          leading: IconButton(
            tooltip: 'Back',
            onPressed: () => Navigator.pop(context),
            icon: StrokeIcon(Glyph.back, color: c.text),
          ),
          // The name changes here, from the menu, so the title follows it.
          title: ListenableBuilder(
            listenable: feeder,
            builder: (context, _) => Text(
              feeder.state.known.name,
              style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
            ),
          ),
          actions: [
            PopupMenuButton<void>(
              tooltip: 'More',
              icon: Icon(Icons.more_vert, color: c.text),
              itemBuilder: (context) => [
                PopupMenuItem(onTap: () => unawaited(_rename(context, feeder)), child: const Text('Rename')),
                PopupMenuItem(onTap: () => unawaited(confirmRestart(context, feeder)), child: const Text('Restart')),
              ],
            ),
            const SizedBox(width: Space.s1),
          ],
          bottom: TabBar(
            labelColor: c.accent,
            unselectedLabelColor: c.textMuted,
            indicatorColor: c.accent,
            indicatorWeight: 3,
            indicatorSize: TabBarIndicatorSize.tab,
            // Four names across a small phone, or in large text, without cutting one off.
            labelPadding: const EdgeInsets.symmetric(horizontal: Space.s1),
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
        body: TabBarView(
          children: [
            TodayTab(feeder: feeder),
            ScheduleTab(feeder: feeder),
            HistoryTab(feeder: feeder),
            SettingsTab(feeder: feeder),
          ],
        ),
      ),
    );
  }
}

/// Ask for a new name. It is this phone's name for the feeder, so it needs
/// no session and works offline.
Future<void> _rename(BuildContext context, FeederController feeder) async {
  final String? name = await askText(
    context,
    title: 'Rename ${feeder.state.known.name}',
    label: 'Name',
    initial: feeder.state.known.name,
    action: 'Rename',
  );
  if (name == null || !context.mounted) return;

  final bool saved = await feeder.rename(name);
  if (!saved && context.mounted) {
    ScaffoldMessenger.of(context).showSnackBar(const SnackBar(content: Text("Couldn't save the name. Try again.")));
  }
}

/// Ask, then restart. The feeder refuses a restart while a feed runs; one
/// that goes unanswered may have gone through, and is not sent again.
Future<void> confirmRestart(BuildContext context, FeederController feeder) async {
  final bool? sure = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text('Restart ${feeder.state.known.name}?'),
      content: const Text("It's back in about a minute. It can't feed during the restart."),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
        FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Restart')),
      ],
    ),
  );
  if (sure != true) return;

  final FeederReply reply = await feeder.ask(const FeederRequest.reboot());
  if (context.mounted && showProblem(context, reply)) {
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text('${feeder.state.known.name} is restarting.')));
  }
}

class _Gone extends StatelessWidget {
  const _Gone();

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(),
    body: const Center(child: Text('This phone no longer knows that feeder.')),
  );
}

/// The feeder a route names, or a screen that says it is gone.
class FeederRoute extends StatelessWidget {
  const FeederRoute({super.key, required this.fleet, required this.identity, this.initialTab = 0});

  final FeederFleet fleet;
  final String identity;
  final int initialTab;

  @override
  Widget build(BuildContext context) {
    final FeederController? feeder = fleet[identity];
    return feeder == null ? const _Gone() : FeederShell(feeder: feeder, initialTab: initialTab);
  }
}
