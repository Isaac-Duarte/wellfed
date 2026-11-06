import 'dart:async';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/home/adopt_prompt.dart';
import 'package:wellfed/screens/home/feeder_card.dart';
import 'package:wellfed/screens/home/todays_meals.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/src/rust/api/install.dart' show PendingInstall;
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/adopt.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

/// Feeder-first Home: one card per feeder this phone knows, large for one
/// feeder and compact for several.
class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key, required this.fleet, required this.earlierInstall, this.adopt, this.pinsProblem});

  final FeederFleet fleet;

  /// Putting a feeder this phone paired onto the home server, with one.
  final AdoptFlow? adopt;

  /// Why the record of unconfirmed writes did not open: then nothing is sent.
  final PinsProblem? pinsProblem;

  /// What an earlier install left, read from its records. A run that died
  /// before it ended (the app killed, say) has no other way to say so, and a
  /// feeder on bank 0 answers Ready with its stock not yet saved.
  final Future<PendingInstall> Function() earlierInstall;

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  PendingInstall _earlier = PendingInstall.nothing;

  @override
  void initState() {
    super.initState();
    unawaited(_readEarlier());
  }

  Future<void> _readEarlier() async {
    final PendingInstall earlier = await widget.earlierInstall();
    if (mounted) setState(() => _earlier = earlier);
  }

  Future<void> _openInstall() async {
    await context.push('/install');
    await _readEarlier();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.fleet,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final List<FeederController> feeders = widget.fleet.all.toList();
      final one = feeders.length == 1;
      return Scaffold(
        appBar: AppBar(
          toolbarHeight: 64,
          titleSpacing: 20,
          title: Text(
            'Feeders',
            style: TextStyle(fontSize: 22, fontWeight: Fonts.semibold, color: c.text),
          ),
          actions: [
            IconButton(
              tooltip: 'Add a feeder',
              icon: StrokeIcon(Glyph.add, color: c.text),
              onPressed: () => context.push('/add'),
            ),
            const SettingsButton(),
            const SizedBox(width: Space.s2),
          ],
        ),
        body: ListView(
          padding: const EdgeInsets.fromLTRB(16, 4, 16, 16),
          children: [
            if (widget.pinsProblem case final PinsProblem problem) ...[
              Notice(
                tone: Tone.fault,
                lead: 'Feeding and changes are off.',
                text: switch (problem) {
                  PinsProblem.inUse => 'Another copy of Wellfed holds its record of unconfirmed writes. Close it, then open the app again.',
                  PinsProblem.unreadable =>
                    "This phone's record of unconfirmed writes can't be read, so the app sends nothing rather than "
                        "risk a feed twice. Clear the app's data (Android Settings, Apps, Wellfed, Storage), then "
                        'pair your feeders again.',
                },
              ),
              const SizedBox(height: Space.s4),
            ],
            if (_earlier != PendingInstall.nothing) ...[
              const Notice(
                tone: Tone.warning,
                lead: "An install didn't finish.",
                text:
                    'Its last steps may not have run, and the original firmware '
                    'may not be saved. Finish it before you rely on this feeder.',
              ),
              const SizedBox(height: Space.s2),
              FilledButton(onPressed: () => unawaited(_openInstall()), child: const Text('Finish installing')),
              const SizedBox(height: Space.s4),
            ],
            if (widget.adopt case final AdoptFlow adopt?)
              for (final KnownFeeder feeder in widget.fleet.adoptable) AdoptPrompt(feeder: feeder, flow: adopt),
            for (final (i, controller) in feeders.indexed) ...[
              if (i > 0) const SizedBox(height: Space.s3),
              FeederCard(controller: controller, compact: !one),
            ],
            if (one) TodaysMeals(controller: feeders.first),
          ],
        ),
      );
    },
  );
}
