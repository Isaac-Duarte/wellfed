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

/// Where a callout's leader runs: out to an edge, or down under the drawing
/// for parts too near an edge to lead out sideways.
enum CalloutSide { left, right, below }

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
    this.crop,
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

  /// The part of the drawing to show, in its units; all of it when null.
  final Rect? crop;

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

/// The widest a label under the drawing gets.
const double _belowWidth = 120;

/// The lift of a label's last line off its leader.
const double _lift = 3;

/// More lines than this and a label reads as a paragraph: it goes to the list.
const int _maxLines = 4;

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
        final Rect shown = drawing.crop ?? lines.box;
        final double width = box.hasBoundedWidth ? box.maxWidth : shown.width * height / shown.height;
        final size = Size(width, height);
        // Labels under the drawing take a band off its height.
        final double band = drawing.callouts
            .where((callout) => callout.side == CalloutSide.below)
            .map((callout) => _measure(callout, c, style, scaler, direction, _belowWidth).height + 2 * _gap)
            .fold<double>(0, math.max);
        final frame = _Frame(lines, shown, Size(width, math.max(1.0, height - band)));
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
      final Rect rect;
      if (callout.side == CalloutSide.below) {
        final Size text = _measure(callout, c, style, scaler, direction, _belowWidth);
        final double left = (at.dx - text.width / 2).clamp(0.0, math.max(0.0, size.width - text.width));
        rect = Rect.fromLTWH(left, frame.bottom + _gap, text.width, text.height);
        if (rect.bottom > size.height + 1) return null;
      } else {
        final bool left = callout.side == CalloutSide.left;
        final double room = (left ? frame.left : size.width - frame.right) - _gap;
        if (room < _minRoom) return null;
        final Size text = _measure(callout, c, style, scaler, direction, room);
        final double top = at.dy - _lift - text.height;
        if (_lineCount(text, style, scaler) > _maxLines || top < 0) return null;
        rect = Rect.fromLTWH(left ? 0 : size.width - room, top, room, text.height);
      }
      final bool crowded = placed.any(
        (other) =>
            other.callout.side == callout.side &&
            switch (callout.side) {
              CalloutSide.below => other.rect.inflate(_gap / 2).overlaps(rect.inflate(_gap / 2)),
              CalloutSide.left || CalloutSide.right => other.rect.top < at.dy + _lift && rect.top < other.at.dy + _lift,
            },
      );
      if (crowded) return null;
      placed.add(_Placed(callout, at, rect));
    }
    return placed;
  }

  /// A label's size as it will draw, wrapped to `maxWidth`.
  static Size _measure(
    Callout callout,
    ThemeTokens c,
    TextStyle style,
    TextScaler scaler,
    TextDirection direction,
    double maxWidth,
  ) {
    final painter = TextPainter(
      text: TextSpan(text: callout.lines.join('\n'), style: _labelStyle(c, callout, style)),
      textDirection: direction,
      textScaler: scaler,
    )..layout(maxWidth: maxWidth);
    final Size size = painter.size;
    painter.dispose();
    return size;
  }

  /// Lines in a measured label, from its height.
  static int _lineCount(Size text, TextStyle style, TextScaler scaler) =>
      (text.height / scaler.scale((style.fontSize ?? Sizes.sm) * (style.height ?? 1))).round();
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
    final TextStyle own = _labelStyle(context.colors, callout, style);
    final TextAlign align = switch (callout.side) {
      CalloutSide.left => TextAlign.left,
      CalloutSide.right => TextAlign.right,
      CalloutSide.below => TextAlign.center,
    };
    final text = Text(callout.lines.join('\n'), textAlign: align, style: own);
    final Rect rect = placed.rect;
    final Offset at = placed.at;
    final VoidCallback? onTap = callout.onTap;
    if (onTap == null) {
      return Positioned(
        left: rect.left,
        top: rect.top,
        width: rect.width,
        child: Semantics(container: true, child: text),
      );
    }

    // The words and their whole leader, grown to the 48 dp a finger needs.
    final Rect reach = switch (callout.side) {
      CalloutSide.left => Rect.fromLTRB(0, rect.top - _lift, at.dx, at.dy + _gap),
      CalloutSide.right => Rect.fromLTRB(at.dx, rect.top - _lift, size.width, at.dy + _gap),
      CalloutSide.below => Rect.fromLTRB(rect.left, at.dy - _gap, rect.right, rect.bottom + _lift),
    };
    final region = Rect.fromCenter(
      center: reach.center,
      width: math.max(48, reach.width),
      height: math.max(48, reach.height),
    );
    return Positioned.fromRect(
      rect: region,
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
                Positioned(left: rect.left - region.left, top: rect.top - region.top, width: rect.width, child: text),
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
