import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/feeder_drawing.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/settings_list.dart';

/// The ways in, the common one first: most feeders arrive on the vendor's
/// firmware. A reset or a lost owner is rare, so those sit below as plain rows.
class AddFeederScreen extends StatelessWidget {
  const AddFeederScreen({super.key});

  @override
  Widget build(BuildContext context) => StepScaffold(
    title: 'Add a feeder',
    body: 'How is this feeder getting to you?',
    content: [
      _InstallCard(onTap: () => context.push('/install')),
      const SizedBox(height: Space.s3),
      RouteCard(
        glyph: Glyph.add,
        title: 'Join with an invite',
        body: 'Someone who owns a feeder shared a code or a link with you.',
        onTap: () => context.push('/join'),
      ),
      const SizedBox(height: Space.s5),
      SettingsSection(
        'Other ways in',
        last: true,
        Column(
          children: [
            SettingsLink(
              'Set up a reset feeder',
              'Already runs Wellfed, but was reset or lost its Wi-Fi',
              first: true,
              onTap: () => context.push('/setup'),
            ),
            SettingsLink(
              'Recover a feeder',
              'On your Wi-Fi, but no phone can open it',
              onTap: () => context.push('/recover'),
            ),
          ],
        ),
      ),
    ],
  );
}

/// The common case, led by the feeder it is for.
class _InstallCard extends StatelessWidget {
  const _InstallCard({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Panel(
      onTap: onTap,
      highlight: true,
      padding: const EdgeInsets.fromLTRB(Space.s3, Space.s4, Space.s4, Space.s4),
      child: Row(
        children: [
          const SizedBox(width: 84, child: FeederDrawing(height: 112)),
          const SizedBox(width: Space.s3),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Install Wellfed',
                  style: TextStyle(fontSize: Sizes.lg, fontWeight: Fonts.semibold, color: c.text),
                ),
                const SizedBox(height: Space.s1),
                Text(
                  "For a feeder still on the vendor's app. This $deviceName replaces its firmware and keeps a backup "
                  'of the original.',
                  style: TextStyle(fontSize: Sizes.base, height: 1.45, color: c.textMuted),
                ),
              ],
            ),
          ),
          const SizedBox(width: Space.s2),
          StrokeIcon(Glyph.chevron, size: 20, color: c.textMuted),
        ],
      ),
    );
  }
}
