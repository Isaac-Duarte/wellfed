import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/feeder_drawing.dart';
import 'package:wellfed/widgets/layout.dart';

/// What Wellfed is pinned to on the feeder: the website hero's claims and
/// anchors (src/web/apps/site/src/lib/sections/Hero.svelte), copied because
/// the site and the app share no code. Each is one the README and the MIT
/// license already make.
const List<({String anchor, String label, CalloutSide side})> _claims = [
  (anchor: 'window', label: 'Open source', side: CalloutSide.right),
  (anchor: 'button_lock', label: 'No account', side: CalloutSide.left),
  (anchor: 'outlet', label: 'No vendor cloud', side: CalloutSide.left),
  (anchor: 'bowl', label: 'Free (MIT)', side: CalloutSide.right),
];

/// The tallest the drawing gets, and the room kept under it for the claims
/// when a phone is too narrow to pin them beside it.
const double _drawingHeight = 340;
const double _claimsRoom = 56;

class WelcomeScreen extends StatelessWidget {
  const WelcomeScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(28, Space.s7, 28, 28),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  SvgPicture.asset(
                    context.isDark ? 'assets/brand/mark.svg' : 'assets/brand/mark-on-light.svg',
                    width: 36,
                    height: 36,
                    excludeFromSemantics: true,
                  ),
                  const SizedBox(width: 10),
                  Text(
                    'Wellfed',
                    style: TextStyle(fontSize: Sizes.lg, fontWeight: Fonts.display, letterSpacing: -0.4, color: c.text),
                  ),
                ],
              ),
              Expanded(
                child: LayoutBuilder(
                  builder: (context, box) => Center(
                    child: SingleChildScrollView(
                      child: FeederDrawing(
                        height: math.max(
                          120,
                          math.min(_drawingHeight, box.maxHeight - MediaQuery.textScalerOf(context).scale(_claimsRoom)),
                        ),
                        label: 'Drawing of the feeder',
                        callouts: [
                          for (final claim in _claims)
                            Callout(anchor: claim.anchor, side: claim.side, lines: [claim.label], color: c.textMuted),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
              const Headline("Your pet's dinner shouldn't need the internet.", size: 36),
              const SizedBox(height: Space.s3),
              Text(
                'Wellfed talks to your feeder over your own Wi-Fi. No '
                'account, no vendor cloud.',
                style: TextStyle(fontSize: Sizes.md, height: 1.5, color: c.textMuted),
              ),
              const SizedBox(height: Space.s5),
              FilledButton(onPressed: () => context.push('/add'), child: const Text('Add a feeder')),
              const SizedBox(height: Space.s3),
              Text(
                'Independent community project. Not affiliated with or '
                'endorsed by PETLIBRO.',
                textAlign: TextAlign.center,
                style: TextStyle(fontSize: Sizes.xs, height: 1.5, color: c.textMuted),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
