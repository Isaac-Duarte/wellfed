import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:wellfed/screens/feeder/clock_portion.dart';
import 'package:wellfed/screens/feeder/diagnostics.dart';
import 'package:wellfed/screens/feeder/people.dart';
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/screens/feeder/updates.dart';
import 'package:wellfed/screens/feeder/wifi.dart';
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

    Widget link(String name, String? note, Widget screen, {bool first = false}) => SettingsTile(
      first: first,
      onTap: () => _open(context, screen),
      child: Row(
        children: [
          Expanded(child: SettingsWords(name, note)),
          StrokeIcon(Glyph.chevron, size: 20, color: context.colors.textMuted),
        ],
      ),
    );

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionLabel('Feeder'),
        Panel(
          child: Column(
            children: [
              link('Wi-Fi', signal, WifiScreen(feeder: feeder), first: true),
              link(
                'Time zone',
                zone == null || zone.isEmpty ? 'Follows the phones' : zone,
                ClockPortionScreen(feeder: feeder),
              ),
              link('People', people, PeopleScreen(feeder: feeder)),
              link('Firmware', null, UpdatesScreen(feeder: feeder)),
              link('Diagnostics', null, DiagnosticsScreen(feeder: feeder)),
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
                'Time zone',
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
