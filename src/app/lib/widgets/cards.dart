import 'package:flutter/material.dart';

import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/icons.dart';

/// A white card with the design's 20 px corners and hairline border.
class Panel extends StatelessWidget {
  const Panel({super.key, required this.child, this.padding = EdgeInsets.zero, this.onTap, this.highlight = false});

  final Widget child;
  final EdgeInsetsGeometry padding;
  final VoidCallback? onTap;

  /// An accent border, for the suggested choice.
  final bool highlight;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final shape = RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(Radii.card),
      side: highlight ? BorderSide(color: c.accent, width: 2) : BorderSide(color: c.line),
    );
    return Material(
      color: c.surface,
      shape: shape,
      clipBehavior: Clip.antiAlias,
      child: onTap == null
          ? Padding(padding: padding, child: child)
          : InkWell(
              onTap: onTap,
              child: Padding(padding: padding, child: child),
            ),
    );
  }
}

/// One way in: an icon tile, a title and what it is for.
class RouteCard extends StatelessWidget {
  const RouteCard({super.key, required this.glyph, required this.title, required this.body, required this.onTap});

  final Glyph glyph;
  final String title;
  final String body;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Panel(
      onTap: onTap,
      padding: const EdgeInsets.all(Space.s4),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          IconTile(glyph),
          const SizedBox(width: Space.s4),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  style: TextStyle(fontSize: Sizes.md, fontWeight: Fonts.semibold, color: c.text),
                ),
                const SizedBox(height: Space.s1),
                Text(
                  body,
                  style: TextStyle(fontSize: Sizes.base, height: 1.45, color: c.textMuted),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// A glyph on a soft accent square.
class IconTile extends StatelessWidget {
  const IconTile(this.glyph, {super.key, this.size = 48, this.muted = false});

  final Glyph glyph;
  final double size;
  final bool muted;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: muted ? c.sunken : c.accentSoft,
        borderRadius: BorderRadius.circular(Radii.icon),
      ),
      alignment: Alignment.center,
      child: StrokeIcon(glyph, size: size / 2, color: muted ? c.textMuted : c.accent),
    );
  }
}

/// Label-value rows with hairlines between, on a card or sunk into one.
class FactList extends StatelessWidget {
  const FactList(this.rows, {super.key, this.sunken = false});

  final List<(String, Widget)> rows;
  final bool sunken;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Container(
      decoration: BoxDecoration(
        color: sunken ? c.sunken : c.surface,
        borderRadius: BorderRadius.circular(sunken ? Radii.icon : Radii.card),
        border: sunken ? null : Border.all(color: c.line),
      ),
      child: Column(
        children: [
          for (final (i, (label, value)) in rows.indexed)
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
              decoration: BoxDecoration(
                border: i == 0 ? null : Border(top: BorderSide(color: c.line)),
              ),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      label,
                      style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                    ),
                  ),
                  DefaultTextStyle.merge(
                    style: TextStyle(fontSize: Sizes.base, color: c.text),
                    child: value,
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

/// Titled rows on a card: "Other reasons setup stops".
class ReasonList extends StatelessWidget {
  const ReasonList(this.rows, {super.key});

  final List<(String, String)> rows;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Panel(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final (i, (title, body)) in rows.indexed)
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
              decoration: BoxDecoration(
                border: i == 0 ? null : Border(top: BorderSide(color: c.line)),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    title,
                    style: TextStyle(fontSize: 15, fontWeight: Fonts.semibold, color: c.text),
                  ),
                  const SizedBox(height: 2),
                  Text(
                    body,
                    style: TextStyle(fontSize: Sizes.base, height: 1.45, color: c.textMuted),
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

/// An uppercase section label.
class SectionLabel extends StatelessWidget {
  const SectionLabel(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) => Text(
    text.toUpperCase(),
    style: TextStyle(
      fontSize: Sizes.sm,
      fontWeight: Fonts.semibold,
      letterSpacing: 0.06 * Sizes.sm,
      color: context.colors.textMuted,
    ),
  );
}

/// A form field with its label above and a hint (and counter) below.
class LabeledField extends StatelessWidget {
  const LabeledField({super.key, required this.label, required this.child, this.hint, this.counter});

  final String label;
  final Widget child;
  final String? hint;
  final String? counter;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final small = TextStyle(fontSize: Sizes.sm, color: c.textMuted);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(label, style: small.copyWith(fontWeight: Fonts.semibold)),
        const SizedBox(height: 6),
        child,
        if (hint != null || counter != null) ...[
          const SizedBox(height: 6),
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(child: Text(hint ?? '', style: small)),
              if (counter case final counter?)
                Text(counter, style: small.copyWith(fontFeatures: const [FontFeature.tabularFigures()])),
            ],
          ),
        ],
      ],
    );
  }
}

/// The feeder render for the current theme.
class FeederImage extends StatelessWidget {
  const FeederImage({super.key, required this.size, this.dimmed = false});

  final double size;
  final bool dimmed;

  @override
  Widget build(BuildContext context) => Opacity(
    opacity: dimmed ? 0.5 : 1,
    child: Image.asset(
      context.isDark ? 'assets/renders/card-dark-cutout.png' : 'assets/renders/card-cutout.png',
      width: size,
      height: size,
      fit: BoxFit.contain,
      excludeFromSemantics: true,
    ),
  );
}
