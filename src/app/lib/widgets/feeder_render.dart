part of 'feeder_drawing.dart';

/// The render laid into the widget: scaled to fit, centered. Its units are
/// fractions of the square render, as the render's hotspots are.
class _Frame {
  _Frame(this.render, this.box, this.size) : scale = math.min(size.height / box.height, size.width / box.width);

  final _Render render;

  /// The part of the render shown, in its units.
  final Rect box;
  final Size size;

  /// Widget pixels per render unit: the whole render's width on screen.
  final double scale;

  Offset get origin => Offset((size.width - box.width * scale) / 2, (size.height - box.height * scale) / 2);

  double get left => origin.dx;
  double get right => origin.dx + box.width * scale;
  double get bottom => origin.dy + box.height * scale;

  Offset at(Offset point) => origin + (point - box.topLeft) * scale;

  /// Where the whole render sits, which is larger than the part shown.
  Rect get image => Rect.fromLTWH(at(Offset.zero).dx, at(Offset.zero).dy, scale, scale);

  /// The part shown, in widget pixels.
  Rect get shown => Rect.fromLTRB(left, origin.dy, right, bottom);

  Offset? anchor(String name) => switch (render.anchors[name]) {
    null => null,
    final Offset point => at(point),
  };
}

/// The rings on highlighted parts and the leaders out to the labels, over
/// the render.
class _Painter extends CustomPainter {
  _Painter({required this.frame, required this.colors, required this.highlights, required this.placed});

  final _Frame frame;
  final ThemeTokens colors;
  final Set<String> highlights;
  final List<_Placed> placed;

  @override
  void paint(Canvas canvas, Size size) {
    final ring = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2
      ..color = colors.accent;
    for (final String name in highlights) {
      final Offset? at = frame.anchor(name);
      if (at == null) continue;
      canvas
        ..drawCircle(at, 12, Paint()..color = colors.accent.withValues(alpha: 0.16))
        ..drawCircle(at, 12, ring);
    }

    final leader = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = _stroke
      ..strokeCap = StrokeCap.round
      ..color = colors.lineStrong;
    final dotFill = Paint()..color = colors.background;
    final dotLine = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = _stroke
      ..color = colors.text;
    for (final _Placed callout in placed) {
      final Offset end = switch (callout.callout.side) {
        CalloutSide.left => Offset(0, callout.at.dy),
        CalloutSide.right => Offset(size.width, callout.at.dy),
        CalloutSide.below => Offset(callout.at.dx, callout.rect.top - _lift),
      };
      canvas
        ..drawLine(callout.at, end, leader)
        ..drawCircle(callout.at, 3.5, dotFill)
        ..drawCircle(callout.at, 3.5, dotLine);
    }
  }

  @override
  bool shouldRepaint(_Painter old) =>
      old.frame.scale != frame.scale ||
      old.frame.size != frame.size ||
      old.colors != colors ||
      old.highlights != highlights ||
      old.placed.length != placed.length ||
      Iterable<int>.generate(placed.length).any((i) => old.placed[i].at != placed[i].at);
}

/// A render's named parts, read once per run from the hotspots the render
/// script wrote beside it.
class _Render {
  const _Render(this.anchors);

  factory _Render.parse(String text) {
    if (jsonDecode(text) case {'hotspots': final Map<String, Object?> hotspots}) {
      return _Render({
        for (final MapEntry(:key, :value) in hotspots.entries)
          if (value case {'x': final num x, 'y': final num y}) key: Offset(x.toDouble(), y.toDouble()),
      });
    }
    throw const FormatException('not a render description');
  }

  final Map<String, Offset> anchors;

  static final Map<FeederView, _Render> ready = {};

  static Future<_Render> load(FeederView view) async {
    if (ready[view] case final render?) return render;
    final ByteData data = await rootBundle.load(view.hotspots);
    final render = _Render.parse(utf8.decode(Uint8List.sublistView(data)));
    ready[view] = render;
    return render;
  }
}
