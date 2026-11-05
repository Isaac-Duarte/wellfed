import 'dart:async';

import 'package:clock/clock.dart';
import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/replies.dart';

/// The feeder's record lines as they come (13.3). Owner only: the API has no
/// way to stop a tail, so the feeder goes on pushing until the session ends,
/// and the controller drops what no screen is listening for.
///
/// A record line carries no time, so each shows when this phone heard it.
/// Pausing holds the view still to read; the lines heard meanwhile wait and
/// join it on resume.
class LiveLogScreen extends StatefulWidget {
  const LiveLogScreen({super.key, required this.feeder, this.share});

  final FeederController feeder;

  /// Where sharing goes, in place of the feeder's own.
  final Future<void> Function(String text)? share;
  @override
  State<LiveLogScreen> createState() => _LiveLogScreenState();
}

/// One line heard, or a count of lines the feeder dropped before sending.
class _Line {
  const _Line(this.at, this.text, {this.dropped = 0});

  final DateTime at;
  final String text;
  final int dropped;
}

class _LiveLogScreenState extends State<LiveLogScreen> {
  /// A long tail would otherwise grow for as long as the screen stays open.
  static const _kept = 2000;

  final _lines = <_Line>[];
  final _waiting = <_Line>[];
  final _filter = TextEditingController();
  bool _paused = false;
  String? _problem;

  @override
  void initState() {
    super.initState();
    widget.feeder.hearLog(_heard);
    _filter.addListener(() => setState(() {}));
    unawaited(_start());
  }

  @override
  void dispose() {
    widget.feeder.hearLog(null);
    _filter.dispose();
    super.dispose();
  }

  Future<void> _start() async {
    final FeederReply reply = await widget.feeder.ask(const FeederRequest.tailLog());
    if (mounted) setState(() => _problem = replyProblem(reply));
  }

  void _heard(int dropped, List<String> lines) {
    final DateTime at = clock.now();
    final List<_Line> into = _paused ? _waiting : _lines;
    setState(() {
      if (dropped > 0) into.add(_Line(at, '… $dropped lines dropped', dropped: dropped));
      into.addAll([for (final line in lines) _Line(at, line)]);
      _trim(into);
    });
  }

  void _togglePause() => setState(() {
    _paused = !_paused;
    if (!_paused) {
      _lines.addAll(_waiting);
      _waiting.clear();
      _trim(_lines);
    }
  });

  static void _trim(List<_Line> lines) {
    if (lines.length > _kept) lines.removeRange(0, lines.length - _kept);
  }

  /// The log as text, each line after the time this phone heard it.
  String _text() => [
    for (final line in [..._lines, ..._waiting]) '${_clock(line.at)} ${line.text}',
  ].join('\n');

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final String filter = _filter.text.trim().toLowerCase();
    final List<_Line> shown = filter.isEmpty
        ? _lines
        : [
            for (final line in _lines)
              if (line.text.toLowerCase().contains(filter)) line,
          ];

    return Scaffold(
      appBar: AppBar(
        toolbarHeight: 64,
        automaticallyImplyLeading: false,
        titleSpacing: 4,
        leading: IconButton(
          tooltip: 'Back',
          onPressed: () => Navigator.pop(context),
          icon: StrokeIcon(Glyph.back, color: c.text),
        ),
        title: Text(
          'Live log',
          style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
        ),
        actions: [
          IconButton(
            tooltip: _paused ? 'Resume' : 'Pause',
            onPressed: _problem == null ? _togglePause : null,
            icon: StrokeIcon(_paused ? Glyph.play : Glyph.pause, color: c.text),
          ),
          IconButton(
            tooltip: 'Share the log',
            onPressed: _lines.isEmpty && _waiting.isEmpty
                ? null
                : () => unawaited((widget.share ?? widget.feeder.share)(_text())),
            icon: Icon(Icons.share_outlined, color: c.text),
          ),
        ],
      ),
      body: SafeArea(
        top: false,
        child: switch (_problem) {
          final String problem? => Padding(
            padding: const EdgeInsets.all(Space.s4),
            child: Notice(tone: Tone.warning, glyph: Glyph.warning, text: problem),
          ),
          null => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _Status(
                paused: _paused,
                heard: _lines.where((line) => line.dropped == 0).length,
                waiting: _waiting.where((line) => line.dropped == 0).length,
                onResume: _togglePause,
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(Space.s4, 0, Space.s4, Space.s3),
                child: TextField(
                  controller: _filter,
                  decoration: InputDecoration(
                    isDense: true,
                    hintText: 'Filter, like PLNET1 or rssi',
                    prefixIcon: Padding(
                      padding: const EdgeInsets.all(12),
                      child: StrokeIcon(Glyph.search, size: 18, color: c.textMuted),
                    ),
                    suffixIcon: _filter.text.isEmpty
                        ? null
                        : IconButton(
                            tooltip: 'Clear the filter',
                            onPressed: _filter.clear,
                            icon: StrokeIcon(Glyph.close, size: 18, color: c.textMuted),
                          ),
                  ),
                ),
              ),
              Expanded(
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(Space.s4, 0, Space.s4, Space.s4),
                  child: Panel(
                    child: _lines.isEmpty
                        ? const Center(
                            child: Padding(
                              padding: EdgeInsets.all(Space.s4),
                              child: Searching("Waiting for the feeder's next line…"),
                            ),
                          )
                        : shown.isEmpty
                        ? Center(
                            child: Text(
                              'No line has “${_filter.text.trim()}”.',
                              style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                            ),
                          )
                        // Newest at the bottom, and the view stays there as
                        // lines come, unless the owner scrolled up to read.
                        : ListView.builder(
                            reverse: true,
                            padding: const EdgeInsets.symmetric(vertical: Space.s2),
                            itemCount: shown.length,
                            itemBuilder: (context, i) => _LogRow(shown[shown.length - 1 - i]),
                          ),
                  ),
                ),
              ),
            ],
          ),
        },
      ),
    );
  }
}

/// Whether lines are coming in, and how many.
class _Status extends StatelessWidget {
  const _Status({required this.paused, required this.heard, required this.waiting, required this.onResume});

  final bool paused;
  final int heard;
  final int waiting;
  final VoidCallback onResume;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final small = TextStyle(fontSize: Sizes.sm, color: c.textMuted);
    return Padding(
      padding: const EdgeInsets.fromLTRB(Space.s4, Space.s1, Space.s2, Space.s2),
      child: Row(
        children: [
          Container(
            width: 8,
            height: 8,
            decoration: BoxDecoration(color: paused ? c.warning : c.ok, shape: BoxShape.circle),
          ),
          const SizedBox(width: Space.s2),
          Expanded(
            child: Text(
              paused
                  ? (waiting == 0 ? 'Paused' : 'Paused · $waiting new ${waiting == 1 ? 'line' : 'lines'}')
                  : 'Streaming while this screen is open',
              style: TextStyle(fontSize: Sizes.sm, fontWeight: Fonts.medium, color: c.text),
            ),
          ),
          if (paused && waiting > 0)
            TextButton(onPressed: onResume, child: const Text('Show them'))
          else
            Padding(
              padding: const EdgeInsets.only(right: Space.s2),
              child: Text('$heard ${heard == 1 ? 'line' : 'lines'}', style: small),
            ),
        ],
      ),
    );
  }
}

/// One record line: when it came, its tag, then its words and `key=value`
/// fields, the keys quieter than their values.
class _LogRow extends StatelessWidget {
  const _LogRow(this.line);

  final _Line line;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    const mono = TextStyle(fontFamily: Fonts.mono, fontSize: 12, height: 1.5);

    if (line.dropped > 0) {
      return Padding(
        padding: const EdgeInsets.symmetric(horizontal: Space.s3, vertical: Space.s1),
        child: Row(
          children: [
            Expanded(child: Divider(color: c.warning.withValues(alpha: 0.4))),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: Space.s2),
              child: Text(line.text, style: mono.copyWith(color: c.warning)),
            ),
            Expanded(child: Divider(color: c.warning.withValues(alpha: 0.4))),
          ],
        ),
      );
    }

    final List<String> words = line.text.split(' ');
    final bool tagged = words.length > 1 && RegExp(r'^[A-Z][A-Z0-9]*$').hasMatch(words.first);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: Space.s3, vertical: 1),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(_clock(line.at), style: mono.copyWith(color: c.textMuted.withValues(alpha: 0.7))),
          const SizedBox(width: Space.s2),
          Expanded(
            child: Text.rich(
              TextSpan(
                style: mono.copyWith(color: c.text),
                children: [
                  for (final (i, word) in words.indexed) ...[
                    if (i > 0) const TextSpan(text: ' '),
                    if (i == 0 && tagged)
                      TextSpan(
                        text: word,
                        style: TextStyle(color: c.accent, fontWeight: Fonts.semibold),
                      )
                    else if (word.indexOf('=') case final int at when at > 0) ...[
                      TextSpan(
                        text: word.substring(0, at + 1),
                        style: TextStyle(color: c.textMuted),
                      ),
                      TextSpan(text: word.substring(at + 1)),
                    ] else
                      TextSpan(text: word),
                  ],
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// `15:04:05`: lines come seconds apart, so the seconds matter.
String _clock(DateTime at) => [at.hour, at.minute, at.second].map((part) => part.toString().padLeft(2, '0')).join(':');
