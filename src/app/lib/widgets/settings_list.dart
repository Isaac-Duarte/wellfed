import 'package:flutter/material.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';

/// A labelled panel of rows, with the room around it every section shares.
class SettingsSection extends StatelessWidget {
  const SettingsSection(this.label, this.child, {super.key, this.last = false});

  final String label;
  final Widget child;
  final bool last;

  @override
  Widget build(BuildContext context) => Padding(
    padding: EdgeInsets.only(bottom: last ? 0 : Space.s5),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.only(left: Space.s1, bottom: Space.s2),
          child: SectionLabel(label),
        ),
        Panel(child: child),
      ],
    ),
  );
}

class SettingsTile extends StatelessWidget {
  const SettingsTile({super.key, required this.first, required this.child, this.onTap});

  final bool first;
  final Widget child;
  final VoidCallback? onTap;

  @override
  // A node of its own: with no tap to tell them apart, as offline, every row
  // on the panel would merge into one.
  Widget build(BuildContext context) => Semantics(
    container: true,
    child: InkWell(
      onTap: onTap,
      child: Container(
        constraints: const BoxConstraints(minHeight: 60),
        padding: const EdgeInsets.symmetric(horizontal: Space.s4, vertical: Space.s3),
        decoration: BoxDecoration(
          border: first ? null : Border(top: BorderSide(color: context.colors.line)),
        ),
        child: child,
      ),
    ),
  );
}

/// A name, and what it stands at under it.
class SettingsWords extends StatelessWidget {
  const SettingsWords(this.name, [this.note, Key? key]) : super(key: key);

  final String name;
  final String? note;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(name, style: TextStyle(fontSize: 15, color: c.text)),
        if (note case final note?) ...[
          const SizedBox(height: 2),
          Text(
            note,
            style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
          ),
        ],
      ],
    );
  }
}

/// A row that opens something: a name, a line under it, and a chevron.
class SettingsLink extends StatelessWidget {
  const SettingsLink(this.name, this.note, {super.key, required this.onTap, this.first = false});

  final String name;
  final String? note;
  final VoidCallback onTap;
  final bool first;

  @override
  Widget build(BuildContext context) => SettingsTile(
    first: first,
    onTap: onTap,
    child: Row(
      children: [
        Expanded(child: SettingsWords(name, note)),
        const SizedBox(width: Space.s2),
        StrokeIcon(Glyph.chevron, size: 20, color: context.colors.textMuted),
      ],
    ),
  );
}
