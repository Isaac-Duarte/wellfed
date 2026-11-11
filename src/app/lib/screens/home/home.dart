import 'dart:async';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/home/adopt_prompt.dart';
import 'package:wellfed/screens/home/feeder_card.dart';
import 'package:wellfed/screens/home/todays_meals.dart';
import 'package:wellfed/screens/server/server_words.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/src/rust/api/install.dart' show PendingInstall;
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/adopt.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/server/server.dart' show OtherDevice;
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

/// Feeder-first Home: one card per feeder this phone knows, large for one
/// feeder and compact for several.
class HomeScreen extends StatefulWidget {
  const HomeScreen({
    super.key,
    required this.fleet,
    required this.earlierInstall,
    this.adopt,
    this.pinsProblem,
    this.askToNotify,
  });

  final FeederFleet fleet;

  /// Putting a feeder this phone paired onto the home server, with one.
  final AdoptFlow? adopt;

  /// Why the record of unconfirmed writes did not open: then nothing is sent.
  /// One that opened but did not read was set aside, and the start said so.
  final PinsProblem? pinsProblem;

  /// What an earlier install left, read from its records. A run that died
  /// before it ended (the app killed, say) has no other way to say so, and a
  /// feeder on bank 0 answers Ready with its stock not yet saved.
  final Future<PendingInstall> Function() earlierInstall;

  /// Asks Android once to let alerts through, as Home shows a feeder of the
  /// phone's own: the moment the owner sees what they would hear from. Home
  /// offers it as it opens and as its feeders change, until it says the
  /// offer is settled. Where alerts run.
  final Future<bool> Function()? askToNotify;

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  PendingInstall _earlier = PendingInstall.nothing;

  @override
  void initState() {
    super.initState();
    unawaited(_readEarlier());
    if (widget.askToNotify != null) {
      widget.fleet.addListener(_offer);
      WidgetsBinding.instance.addPostFrameCallback((_) => _offer());
    }
  }

  @override
  void dispose() {
    widget.fleet.removeListener(_offer);
    super.dispose();
  }

  /// Settled once the callback says so, and never two at once.
  bool _offered = false;
  bool _offering = false;

  void _offer() {
    final Future<bool> Function()? ask = widget.askToNotify;
    if (ask == null || _offered || _offering) return;
    _offering = true;
    unawaited(
      ask().then((settled) {
        _offering = false;
        _offered = settled;
        if (settled) widget.fleet.removeListener(_offer);
      }),
    );
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
      // What needs the owner before the feeders: a write record that can't be
      // trusted, an unfinished install, feeders to put on the server.
      final List<Widget> notices = [
        if (widget.pinsProblem case final PinsProblem problem) ...[
          Notice(
            tone: Tone.fault,
            lead: 'Feeding and changes are off.',
            text: switch (problem) {
              PinsProblem.inUse =>
                'Another copy of Wellfed holds its record of unconfirmed writes. Close it, then open the app again.',
              PinsProblem.unavailable =>
                "This $deviceName's storage couldn't open its record of unconfirmed writes, so the app sends "
                    'nothing rather than risk a feed twice. Close the app and open it again; if this stays, restart '
                    'the $deviceName.',
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
        for (final OtherDevice other in widget.fleet.others) ...[
          Notice(tone: Tone.neutral, text: otherDeviceWords(other)),
          const SizedBox(height: Space.s3),
        ],
      ];
      if (isDesktop) return _DesktopOverview(feeders: feeders, notices: notices);

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
            ...notices,
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

/// Home on a desktop: the sidebar already adds feeders and opens settings,
/// so the page is only the feeders. One sits beside its meals for today;
/// several tile two to a row.
class _DesktopOverview extends StatelessWidget {
  const _DesktopOverview({required this.feeders, required this.notices});

  final List<FeederController> feeders;
  final List<Widget> notices;

  @override
  Widget build(BuildContext context) {
    final bool one = feeders.length == 1;
    return Scaffold(
      body: ListView(
        padding: const EdgeInsets.fromLTRB(Space.s5, Space.s6, Space.s5, Space.s5),
        children: [
          const Headline('Overview'),
          const SizedBox(height: Space.s5),
          ...notices,
          if (one)
            LayoutBuilder(
              builder: (context, box) => box.maxWidth < 720
                  ? Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        FeederCard(controller: feeders.single),
                        TodaysMeals(controller: feeders.single),
                      ],
                    )
                  : Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Expanded(child: FeederCard(controller: feeders.single)),
                        const SizedBox(width: Space.s5),
                        // Beside the card, not under it, the meals need no gap above.
                        Expanded(child: TodaysMeals(controller: feeders.single, gap: 0)),
                      ],
                    ),
            )
          else
            LayoutBuilder(
              builder: (context, box) {
                final double width = box.maxWidth < 720 ? box.maxWidth : (box.maxWidth - Space.s4) / 2;
                return Wrap(
                  spacing: Space.s4,
                  runSpacing: Space.s4,
                  children: [
                    for (final controller in feeders)
                      SizedBox(
                        width: width,
                        // A window has the room for each feeder's next meal, last
                        // feed and power that a phone folds away.
                        child: FeederCard(controller: controller),
                      ),
                  ],
                );
              },
            ),
        ],
      ),
    );
  }
}
