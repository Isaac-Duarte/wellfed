import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:wellfed/screens/feeder/clock_portion.dart';
import 'package:wellfed/screens/feeder/diagnostics.dart';
import 'package:wellfed/screens/feeder/meal_call.dart';
import 'package:wellfed/screens/feeder/people.dart';
import 'package:wellfed/screens/feeder/today.dart' show clockWords;
import 'package:wellfed/screens/feeder/updates.dart';
import 'package:wellfed/screens/feeder/wifi.dart';
import 'package:wellfed/src/rust/api/feeder.dart' show CallInfo, FeederReply, FeederRequest;
import 'package:wellfed/src/rust/api/model.dart' show CallClip_None, CallClip_Receiving, CallClip_Stored;
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/replies.dart';
import 'package:wellfed/widgets/settings_list.dart';

class SettingsOwnerSection extends StatefulWidget {
  const SettingsOwnerSection({super.key, required this.feeder});

  final FeederController feeder;

  @override
  State<SettingsOwnerSection> createState() => _OwnerSectionState();
}

class _OwnerSectionState extends State<SettingsOwnerSection> {
  FeederController get feeder => widget.feeder;

  @override
  void initState() {
    super.initState();
    if (feeder.state.clients.isEmpty) unawaited(feeder.loadClients());
    // Read each time: another phone may have changed the clip. The row shows
    // the last one read meanwhile.
    unawaited(feeder.loadCall());
  }

  void _open(BuildContext context, Widget screen) =>
      unawaited(Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => screen)));

  @override
  Widget build(BuildContext context) {
    final Card? card = feeder.state.card;
    final String? signal = switch (card?.rssi) {
      null => null,
      >= -60 => 'Strong signal',
      >= -72 => 'Fair signal',
      _ => 'Weak signal',
    };
    final String? people = feeder.state.clients.isEmpty ? null : '${feeder.state.clients.length} of $clientsMost';
    final CallInfo? call = feeder.state.call;
    final String? clip = switch (call?.clip) {
      null => null,
      CallClip_Stored() => 'Your recording · ${((call?.ms ?? 0) / 1000).round()} s',
      CallClip_Receiving() => "An upload didn't finish",
      CallClip_None() => 'None recorded',
    };

    return SettingsSection(
      'Feeder',
      Column(
        children: [
          _Link('Wi-Fi', signal, WifiScreen(feeder: feeder), first: true, open: (screen) => _open(context, screen)),
          _Link(
            'Clock and portion size',
            card == null ? null : clockWords(context, card),
            ClockPortionScreen(feeder: feeder),
            open: (screen) => _open(context, screen),
          ),
          _Link(
            'Meal call',
            clip,
            MealCallScreen(feeder: feeder, pick: feeder.pickRecording),
            open: (screen) => _open(context, screen),
          ),
          _Link('People', people, PeopleScreen(feeder: feeder), open: (screen) => _open(context, screen)),
          _Link(
            'Firmware',
            card == null ? null : shortBuild(card.build),
            UpdatesScreen(feeder: feeder),
            open: (screen) => _open(context, screen),
          ),
          _Link(
            'Diagnostics',
            card == null ? null : 'Up ${upText(card.uptimeMs ~/ 1000)}',
            DiagnosticsScreen(feeder: feeder),
            open: (screen) => _open(context, screen),
          ),
        ],
      ),
    );
  }
}

class SettingsOwnerOnly extends StatelessWidget {
  const SettingsOwnerOnly({super.key});

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return SettingsSection(
      'Owner only',
      Column(
        children: [
          for (final (i, name) in const [
            'Wi-Fi',
            'Clock and portion size',
            'Meal call',
            'Firmware updates',
            'Remove people',
          ].indexed)
            SettingsTile(
              first: i == 0,
              child: Row(
                children: [
                  StrokeIcon(Glyph.lock, size: 18, color: c.textMuted),
                  const SizedBox(width: Space.s3),
                  Text(
                    name,
                    style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

class _Link extends StatelessWidget {
  const _Link(this.name, this.note, this.screen, {required this.open, this.first = false});

  final String name;
  final String? note;
  final Widget screen;
  final void Function(Widget) open;
  final bool first;

  @override
  Widget build(BuildContext context) => SettingsLink(name, note, first: first, onTap: () => open(screen));
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
