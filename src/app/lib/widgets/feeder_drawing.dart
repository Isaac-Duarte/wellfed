import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:path_parsing/path_parsing.dart';
import 'package:wellfed/theme/theme.dart';

part 'feeder_lines.dart';

/// Which traced drawing to show. Both come from assets/model/lineart.py, the
/// same files the website's hero draws, so the app and the site show one
/// feeder.
enum FeederView {
  /// The whole feeder at three quarters, as the feeder cards show it.
  card('assets/renders/card-lines.json'),

  /// Straight on to the control panel, for finding a button or a light.
  panel('assets/renders/panel-lines.json');

  const FeederView(this.asset);

  final String asset;
}

enum CalloutSide { left, right }

/// How much food the hopper shows through its window.
enum FoodShown { full, low, none }

/// A label pinned to a part of the feeder by a leader line.
@immutable
class Callout {
  const Callout({
    required this.anchor,
    required this.lines,
    required this.side,
    this.color,
    this.summary,
    this.onTap,
    this.tapHint,
  });

  /// One of the drawing's anchors: `window`, `bowl`, `outlet`, `button_lock`
  /// and the other buttons, or a `led_*`.
  final String anchor;

  /// Stacked over the leader on the drawing.
  final List<String> lines;

  /// The label as one line, for the list; the lines joined with a dot when
  /// they read as one.
  final String? summary;
  final CalloutSide side;
  final Color? color;
  final VoidCallback? onTap;

  /// What a tap does, for a screen reader: the label says what is, not what
  /// changes.
  final String? tapHint;

  String get text => summary ?? lines.join(' · ');
}

/// The feeder as a technical line drawing in the theme's colors, over a
/// drafting table's dot grid, with callouts led out to either side.
///
/// When a label won't fit beside the drawing in three lines, or two labels on
/// one side would overlap, every label moves into a list under it: the site's
/// hero does the same on a phone.
class FeederDrawing extends StatefulWidget {
  const FeederDrawing({
    super.key,
    this.view = FeederView.card,
    required this.height,
    this.callouts = const [],
    this.highlights = const {},
    this.food = FoodShown.full,
    this.dimmed = false,
    this.label,
  });

  final FeederView view;
  final double height;
  final List<Callout> callouts;

  /// Anchors ringed in the accent color, for "press this one".
  final Set<String> highlights;
  final FoodShown food;
  final bool dimmed;

  /// What the picture shows, for a screen reader. The callouts read out on
  /// their own.
  final String? label;

  @override
  State<FeederDrawing> createState() => _FeederDrawingState();
}

class _FeederDrawingState extends State<FeederDrawing> {
  late Future<_Lines> _lines;

  @override
  void initState() {
    super.initState();
    _lines = _Lines.load(widget.view);
  }

  @override
  void didUpdateWidget(FeederDrawing old) {
    super.didUpdateWidget(old);
    if (old.view != widget.view) _lines = _Lines.load(widget.view);
  }

  @override
  Widget build(BuildContext context) => FutureBuilder<_Lines>(
    future: _lines,
    initialData: _Lines.ready[widget.view],
    builder: (context, snapshot) => switch (snapshot.data) {
      null => SizedBox(height: widget.height),
      final _Lines lines => _Laid(lines: lines, drawing: widget),
    },
  );
}

/// Room between a label and the drawing, and the least room worth a label.
const double _gap = 8;
const double _minRoom = 64;

/// The lift of a label's last line off its leader.
const double _lift = 3;

/// Lines in logical pixels at any scale: the site's strokes are in drawing
/// units, which a phone shrinks past seeing.
const double _stroke = 1.2;

class _Laid extends StatelessWidget {
  const _Laid({required this.lines, required this.drawing});

  final _Lines lines;
  final FeederDrawing drawing;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final TextStyle style = DefaultTextStyle.of(context).style
        .merge(TextStyle(fontSize: Sizes.sm, height: 1.3, fontWeight: Fonts.medium, color: c.text));
    final TextScaler scaler = MediaQuery.textScalerOf(context);
    final TextDirection direction = Directionality.of(context);

    return LayoutBuilder(
      builder: (context, box) {
        final double height = drawing.height;
        final double width = box.hasBoundedWidth ? box.maxWidth : lines.box.width * height / lines.box.height;
        final size = Size(width, height);
        final frame = _Frame(lines, size);
        final List<_Placed>? placed = _place(frame, size, drawing.callouts, c, style, scaler, direction);

        final picture = Opacity(
          opacity: drawing.dimmed ? 0.5 : 1,
          child: RepaintBoundary(
            child: CustomPaint(
              size: size,
              painter: _Painter(
                frame: frame,
                colors: c,
                food: drawing.food,
                highlights: drawing.highlights,
                placed: placed ?? const [],
              ),
            ),
          ),
        );
        final figure = SizedBox(
          width: width,
          height: height,
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              Positioned.fill(
                child: drawing.label == null
                    ? ExcludeSemantics(child: picture)
                    : Semantics(container: true, image: true, label: drawing.label, child: picture),
              ),
              for (final callout in placed ?? const <_Placed>[]) _OnDrawing(callout, style: style, size: size),
            ],
          ),
        );
        if (placed != null) return figure;

        return Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            figure,
            const SizedBox(height: Space.s2),
            Wrap(
              alignment: WrapAlignment.center,
              crossAxisAlignment: WrapCrossAlignment.center,
              spacing: Space.s4,
              children: [for (final callout in drawing.callouts) _Listed(callout, style: style)],
            ),
          ],
        );
      },
    );
  }

  /// Where each label goes beside the drawing, or null for the list.
  static List<_Placed>? _place(
    _Frame frame,
    Size size,
    List<Callout> callouts,
    ThemeTokens c,
    TextStyle style,
    TextScaler scaler,
    TextDirection direction,
  ) {
    final placed = <_Placed>[];
    for (final callout in callouts) {
      final Offset? at = frame.anchor(callout.anchor);
      if (at == null) continue;
      final bool left = callout.side == CalloutSide.left;
      final double room = (left ? frame.left : size.width - frame.right) - _gap;
      if (room < _minRoom) return null;

      final painter = TextPainter(
        text: TextSpan(text: callout.lines.join('\n'), style: _labelStyle(c, callout, style)),
        textDirection: direction,
        textScaler: scaler,
      )..layout(maxWidth: room);
      final int count = painter.computeLineMetrics().length;
      final double textHeight = painter.height;
      painter.dispose();

      final double top = at.dy - _lift - textHeight;
      if (count > 3 || top < 0) return null;
      final rect = Rect.fromLTWH(left ? 0 : size.width - room, top, room, textHeight);
      final bool crowded = placed.any(
        (other) => other.callout.side == callout.side && other.rect.top < at.dy + _gap && rect.top < other.at.dy + _gap,
      );
      if (crowded) return null;
      placed.add(_Placed(callout, at, rect));
    }
    return placed;
  }
}

class _Placed {
  const _Placed(this.callout, this.at, this.rect);

  final Callout callout;

  /// The anchor, in the widget's pixels.
  final Offset at;

  /// The label's box, above its leader at the edge it runs to.
  final Rect rect;
}

/// A label beside the drawing. One that does something gets a 48 dp target
/// over its whole leader, so the finger needn't find the words.
class _OnDrawing extends StatelessWidget {
  const _OnDrawing(this.placed, {required this.style, required this.size});

  final _Placed placed;
  final TextStyle style;
  final Size size;

  @override
  Widget build(BuildContext context) {
    final Callout callout = placed.callout;
    final bool left = callout.side == CalloutSide.left;
    final TextStyle own = _labelStyle(context.colors, callout, style);
    final text = Text(callout.lines.join('\n'), textAlign: left ? TextAlign.left : TextAlign.right, style: own);
    final Rect rect = placed.rect;
    final VoidCallback? onTap = callout.onTap;
    if (onTap == null) {
      return Positioned(
        left: rect.left,
        top: rect.top,
        width: rect.width,
        child: Semantics(container: true, child: text),
      );
    }

    double top = rect.top - _lift;
    double bottom = placed.at.dy + _gap;
    if (bottom - top < 48) {
      final double grow = (48 - (bottom - top)) / 2;
      top -= grow;
      bottom += grow;
    }
    final double regionLeft = left ? 0 : placed.at.dx;
    final double regionWidth = math.max(left ? placed.at.dx : size.width - placed.at.dx, rect.width);
    return Positioned(
      left: left ? regionLeft : size.width - regionWidth,
      top: top,
      width: regionWidth,
      height: bottom - top,
      child: Semantics(
        container: true,
        button: true,
        onTapHint: callout.tapHint,
        child: Material(
          type: MaterialType.transparency,
          child: InkWell(
            onTap: onTap,
            borderRadius: BorderRadius.circular(Radii.control),
            child: Stack(
              children: [
                Positioned(
                  left: left ? 0 : null,
                  right: left ? null : 0,
                  top: rect.top - top,
                  width: rect.width,
                  child: text,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// A label in the list under a drawing too narrow for it.
class _Listed extends StatelessWidget {
  const _Listed(this.callout, {required this.style});

  final Callout callout;
  final TextStyle style;

  @override
  Widget build(BuildContext context) {
    final text = Text(callout.text, textAlign: TextAlign.center, style: _labelStyle(context.colors, callout, style));
    final VoidCallback? onTap = callout.onTap;
    if (onTap == null) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: Space.s1),
        child: text,
      );
    }
    return Semantics(
      container: true,
      button: true,
      onTapHint: callout.tapHint,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(Radii.control),
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 48, minWidth: 48),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Space.s2),
            child: Center(widthFactor: 1, child: text),
          ),
        ),
      ),
    );
  }
}

/// A label that does something reads as a link, in the accent.
TextStyle _labelStyle(ThemeTokens c, Callout callout, TextStyle base) {
  if (callout.onTap != null) return base.copyWith(color: callout.color ?? c.accent, fontWeight: Fonts.semibold);
  return base.copyWith(color: callout.color);
}
