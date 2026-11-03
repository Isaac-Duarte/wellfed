part of 'feeder_drawing.dart';

/// The drawing's units laid into the widget: scaled to fit, centered.
class _Frame {
  _Frame(this.lines, this.box, this.size) : scale = math.min(size.height / box.height, size.width / box.width);

  final _Lines lines;

  /// The part of the drawing shown, in its units.
  final Rect box;
  final Size size;
  final double scale;

  Offset get origin => Offset((size.width - box.width * scale) / 2, (size.height - box.height * scale) / 2);

  double get left => origin.dx;
  double get right => origin.dx + box.width * scale;
  double get bottom => origin.dy + box.height * scale;

  Offset at(Offset point) => origin + (point - box.topLeft) * scale;

  Offset? anchor(String name) => switch (lines.anchors[name]) {
    null => null,
    final Offset point => at(point),
  };
}

class _Painter extends CustomPainter {
  _Painter({
    required this.frame,
    required this.colors,
    required this.food,
    required this.highlights,
    required this.placed,
  });

  final _Frame frame;
  final ThemeTokens colors;
  final FoodShown food;
  final Set<String> highlights;
  final List<_Placed> placed;

  @override
  void paint(Canvas canvas, Size size) {
    _dots(canvas, size);

    final _Lines lines = frame.lines;
    canvas
      ..save()
      ..translate(frame.origin.dx, frame.origin.dy)
      ..scale(frame.scale)
      ..translate(-frame.box.left, -frame.box.top)
      ..clipRect(frame.box);
    for (final _Group group in lines.groups) {
      final (Color color, double weight) = switch ((group.part, group.line)) {
        (_Part.food, _) => (colors.food, 1.25),
        (_Part.icon, _) => (colors.textMuted, 0.75),
        (_Part.window, _) || (_, _Line.crease) => (colors.lineStrong, 1.0),
        (_, _Line.outline || _Line.edge) => (colors.text, 1.15),
      };
      final paint = Paint()
        ..style = PaintingStyle.stroke
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round
        ..color = color
        ..strokeWidth = _stroke * weight / frame.scale;
      if (group.part != _Part.food) {
        canvas.drawPath(group.path, paint);
        continue;
      }
      switch (food) {
        case FoodShown.full:
          canvas.drawPath(group.path, paint);
        case FoodShown.low:
          // The bottom quarter of the window: what a low reading means.
          final Rect window = lines.window ?? group.path.getBounds();
          canvas
            ..save()
            ..clipRect(
              Rect.fromLTRB(lines.box.left, window.bottom - window.height / 4, lines.box.right, lines.box.bottom),
            )
            ..drawPath(group.path, paint)
            ..restore();
        case FoodShown.none:
          break;
      }
    }
    canvas.restore();

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

  /// A drafting table's dot grid, fading out from behind the feeder.
  void _dots(Canvas canvas, Size size) {
    const pitch = 16.0;
    final Offset center = size.center(Offset.zero);
    final paint = Paint();
    for (double y = pitch / 2; y < size.height; y += pitch) {
      for (double x = pitch / 2; x < size.width; x += pitch) {
        final double dx = (x - center.dx) / (size.width / 2);
        final double dy = (y - center.dy) / (size.height / 2);
        final double t = math.sqrt(dx * dx + dy * dy);
        final double fade = t <= 0.35 ? 1 : 1 - (t - 0.35) / 0.65;
        if (fade <= 0) continue;
        paint.color = colors.lineStrong.withValues(alpha: 0.45 * fade);
        canvas.drawCircle(Offset(x, y), 1.1, paint);
      }
    }
  }

  @override
  bool shouldRepaint(_Painter old) =>
      old.frame.scale != frame.scale ||
      old.frame.size != frame.size ||
      old.colors != colors ||
      old.food != food ||
      old.highlights != highlights ||
      old.placed.length != placed.length ||
      Iterable<int>.generate(placed.length).any((i) => old.placed[i].at != placed[i].at);
}

enum _Part { body, bowl, window, icon, food }

/// A silhouette, a fold inside it, or an open boundary.
enum _Line { outline, crease, edge }

class _Group {
  const _Group(this.part, this.line, this.path);

  final _Part part;
  final _Line line;
  final Path path;
}

/// A traced drawing, parsed once per run: the paths don't change while the
/// app is open.
class _Lines {
  const _Lines(this.box, this.anchors, this.groups, this.window);

  factory _Lines.parse(String text) {
    final Object? json = jsonDecode(text);
    if (json case {
      'viewBox': [final num x, final num y, final num w, final num h],
      'anchors': final Map<String, Object?> anchors,
      'groups': final List<Object?> groups,
    }) {
      final Map<String, _Part> parts = _Part.values.asNameMap();
      final Map<String, _Line> kinds = _Line.values.asNameMap();
      final parsed = <_Group>[
        for (final group in groups)
          if (group case {
            'part': final String partName,
            'line': final String lineName,
            'paths': final List<Object?> paths,
          })
            if ((parts[partName], kinds[lineName]) case (final _Part part?, final _Line line?))
              _Group(part, line, _path(paths.whereType<String>())),
      ];
      final Rect? window = parsed
          .where((group) => group.part == _Part.window)
          .map((group) => group.path.getBounds())
          .fold<Rect?>(null, (all, bounds) => all?.expandToInclude(bounds) ?? bounds);
      return _Lines(
        Rect.fromLTWH(x.toDouble(), y.toDouble(), w.toDouble(), h.toDouble()),
        {
          for (final MapEntry(:key, :value) in anchors.entries)
            if (value case [final num ax, final num ay]) key: Offset(ax.toDouble(), ay.toDouble()),
        },
        List.unmodifiable(parsed),
        window,
      );
    }
    throw const FormatException('not a traced drawing');
  }

  final Rect box;
  final Map<String, Offset> anchors;
  final List<_Group> groups;

  /// The hopper's window, which a low reading fills a quarter of.
  final Rect? window;

  static final Map<FeederView, _Lines> ready = {};

  /// Through `load`, not `loadString`: that hands a file over 50 KB to an
  /// isolate, which a widget test's fake clock never lets finish.
  static Future<_Lines> load(FeederView view) async {
    if (ready[view] case final lines?) return lines;
    final ByteData data = await rootBundle.load(view.asset);
    final _Lines lines = _Lines.parse(utf8.decode(Uint8List.sublistView(data)));
    ready[view] = lines;
    return lines;
  }

  static Path _path(Iterable<String> data) {
    final path = Path();
    final proxy = _PathProxy(path);
    for (final d in data) {
      writeSvgPathDataToPath(d, proxy);
    }
    return path;
  }
}

class _PathProxy implements PathProxy {
  _PathProxy(this.path);

  final Path path;

  @override
  void moveTo(double x, double y) => path.moveTo(x, y);

  @override
  void lineTo(double x, double y) => path.lineTo(x, y);

  @override
  void cubicTo(double x1, double y1, double x2, double y2, double x3, double y3) =>
      path.cubicTo(x1, y1, x2, y2, x3, y3);

  @override
  void close() => path.close();
}
