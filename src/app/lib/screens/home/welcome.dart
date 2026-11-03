import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/settings/app_settings.dart' show SettingsButton;
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

/// The drawing takes what the rest of the screen leaves: about this much at
/// normal text, growing with it. Bounded so it stays a drawing.
const double _restOfScreen = 540;
const double _drawingLeast = 160;
const double _drawingMost = 340;

class WelcomeScreen extends StatelessWidget {
  const WelcomeScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Scaffold(
      body: SafeArea(
        // Fills the screen, and scrolls once large text makes it longer.
        child: LayoutBuilder(
          builder: (context, viewport) => SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(28, Space.s7, 28, 28),
            child: ConstrainedBox(
              constraints: BoxConstraints(minHeight: math.max(0, viewport.maxHeight - Space.s7 - 28)),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
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
                      Flexible(
                        child: Text(
                          'Wellfed',
                          style: TextStyle(
                            fontSize: Sizes.lg,
                            fontWeight: Fonts.display,
                            letterSpacing: -0.4,
                            color: c.text,
                          ),
                        ),
                      ),
                      const Spacer(),
                      const SettingsButton(),
                    ],
                  ),
                  Padding(
                    padding: const EdgeInsets.symmetric(vertical: Space.s4),
                    child: FeederDrawing(
                      height: (viewport.maxHeight - MediaQuery.textScalerOf(context).scale(_restOfScreen)).clamp(
                        _drawingLeast,
                        _drawingMost,
                      ),
                      label: 'Drawing of the feeder',
                      callouts: [
                        for (final claim in _claims)
                          Callout(anchor: claim.anchor, side: claim.side, lines: [claim.label], color: c.textMuted),
                      ],
                    ),
                  ),
                  Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
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
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
