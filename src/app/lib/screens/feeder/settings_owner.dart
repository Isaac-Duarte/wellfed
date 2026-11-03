import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:wellfed/screens/feeder/clock_portion.dart';
import 'package:wellfed/screens/feeder/diagnostics.dart';
import 'package:wellfed/screens/feeder/meal_call.dart';
import 'package:wellfed/screens/feeder/people.dart';
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/screens/feeder/updates.dart';
import 'package:wellfed/screens/feeder/wifi.dart';
import 'package:wellfed/src/rust/api/feeder.dart' show CallInfo;
import 'package:wellfed/src/rust/api/model.dart' show CallClip_None, CallClip_Receiving, CallClip_Stored;
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';

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
    final String? zone = card?.time?.zone;
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

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionLabel('Feeder'),
        Panel(
          child: Column(
            children: [
              _Link('Wi-Fi', signal, WifiScreen(feeder: feeder), first: true, open: (screen) => _open(context, screen)),
              _Link(
                'Clock and portion size',
                zone == null || zone.isEmpty ? 'Follows the phones' : zone,
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
        ),
      ],
    );
  }
}

class SettingsOwnerOnly extends StatelessWidget {
  const SettingsOwnerOnly({super.key});

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionLabel('Owner only'),
        Panel(
          child: Column(
            children: [
              for (final (i, name) in const [
                'Wi-Fi',
                'Clock and portion size',
                'Meal call',
                'Firmware updates',
                'Restart, remove people',
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
        ),
      ],
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
  Widget build(BuildContext context) => SettingsTile(
    first: first,
    onTap: () => open(screen),
    child: Row(
      children: [
        Expanded(child: SettingsWords(name, note)),
        StrokeIcon(Glyph.chevron, size: 20, color: context.colors.textMuted),
      ],
    ),
  );
}
