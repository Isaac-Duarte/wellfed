import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/theme/theme.dart';

/// One choice of a few, as equal segments: one row when every label fits,
/// otherwise as few even rows as fit, so no option sits alone on a line of
/// its own at a narrow width or a large text size.
class Segments<T> extends StatelessWidget {
  const Segments(this.options, {super.key, required this.selected, required this.onSelected});

  final List<(String, T)> options;
  final bool Function(T) selected;
  final void Function(T) onSelected;

  static const double _gap = Space.s2;
  static const double _padding = Space.s1;

  static TextStyle style(Color color) => TextStyle(fontSize: Sizes.base, fontWeight: Fonts.semibold, color: color);

  /// How many to a row: every label must fit its segment unbroken.
  int _columns(BuildContext context, double width) {
    final TextScaler scaler = MediaQuery.textScalerOf(context);
    // The font the segment's text inherits, not the engine's default.
    final TextStyle measured = DefaultTextStyle.of(context).style.merge(style(Colors.black));
    double widest = 0;
    for (final (name, _) in options) {
      final painter = TextPainter(
        text: TextSpan(text: name, style: measured),
        textDirection: TextDirection.ltr,
        textScaler: scaler,
      )..layout();
      if (painter.width > widest) widest = painter.width;
      painter.dispose();
    }
    final double cell = widest + 2 * _padding;
    for (var rows = 1; rows < options.length; rows++) {
      final int columns = (options.length / rows).ceil();
      if (columns * cell + (columns - 1) * _gap <= width) return columns;
    }
    return 1;
  }

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final int columns = _columns(context, constraints.maxWidth);

      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (var start = 0; start < options.length; start += columns) ...[
            if (start > 0) const SizedBox(height: _gap),
            Row(
              children: [
                for (var i = start; i < start + columns; i++) ...[
                  if (i > start) const SizedBox(width: _gap),
                  Expanded(
                    child: i < options.length
                        ? Segment(
                            options[i].$1,
                            selected: selected(options[i].$2),
                            onTap: () => onSelected(options[i].$2),
                          )
                        : const SizedBox.shrink(),
                  ),
                ],
              ],
            ),
          ],
        ],
      );
    },
  );
}

/// One option of [Segments]: filled when chosen, as the day toggles are.
class Segment extends StatelessWidget {
  const Segment(this.label, {super.key, required this.selected, required this.onTap});

  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final shape = BorderRadius.circular(Radii.control);

    return Semantics(
      button: true,
      selected: selected,
      inMutuallyExclusiveGroup: true,
      child: Material(
        color: selected ? c.accent : Colors.transparent,
        shape: RoundedRectangleBorder(
          borderRadius: shape,
          side: selected ? BorderSide.none : BorderSide(color: c.lineStrong),
        ),
        child: InkWell(
          borderRadius: shape,
          onTap: onTap,
          child: Container(
            constraints: const BoxConstraints(minHeight: 48),
            padding: const EdgeInsets.symmetric(horizontal: Segments._padding, vertical: Space.s2),
            alignment: Alignment.center,
            child: Text(label, maxLines: 1, softWrap: false, style: Segments.style(selected ? c.accentText : c.text)),
          ),
        ),
      ),
    );
  }
}

/// One day of the week, a letter in a circle.
class DayToggle extends StatelessWidget {
  const DayToggle({super.key, required this.day, required this.on, required this.onTap});

  final Weekday day;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final String letter = day.name[0].toUpperCase();
    return Semantics(
      button: true,
      selected: on,
      label: day.name,
      // The circle is smaller than the target, which is Android's 48dp.
      child: InkWell(
        customBorder: const CircleBorder(),
        onTap: onTap,
        child: SizedBox.square(
          dimension: 48,
          child: Center(
            child: Container(
              width: 40,
              height: 40,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: on ? c.accent : null,
                shape: BoxShape.circle,
                border: on ? null : Border.all(color: c.lineStrong),
              ),
              child: ExcludeSemantics(
                child: Text(
                  letter,
                  style: TextStyle(fontSize: Sizes.base, fontWeight: Fonts.semibold, color: on ? c.accentText : c.text),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
