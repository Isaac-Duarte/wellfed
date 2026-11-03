import 'dart:async';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/src/rust/api/feeder.dart' show CalibrationInfo;
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/when.dart';

/// The Feed now button: a new feed, so a known ending the owner has moved on
/// from gives way to the portions. One that isn't known stays until it is
/// closed, as the owner should look at the bowl before feeding again.
Future<void> feedNow(BuildContext context, FeederController feeder) {
  final bool known = switch (feeder.state.feed) {
    FeedEnded(:final FeedResult result) => !result.uncertain,
    FeedRefused() || FeedNotSent() => true,
    _ => false,
  };
  if (known) feeder.dismissFeed();
  return showFeedSheet(context, feeder);
}

/// Confirm the portions, then follow the feed to its end. The sheet shows
/// only what the feeder said; an uncertain feed shows its range and is never
/// offered again as a retry.
Future<void> showFeedSheet(BuildContext context, FeederController feeder) => showModalBottomSheet<void>(
  context: context,
  isScrollControlled: true,
  barrierColor: context.colors.scrim.withValues(alpha: 0.6),
  builder: (context) => ListenableBuilder(
    listenable: feeder,
    // Scrolls only when the text is too large for the screen.
    builder: (context, _) => SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(Space.s5, 0, Space.s5, 28),
        child: switch (feeder.state.feed) {
          FeedIdle() => _Confirm(feeder: feeder),
          _ => _Progress(feeder: feeder),
        },
      ),
    ),
  ),
);

TextStyle _title(BuildContext context, {double size = 24}) =>
    TextStyle(fontSize: size, fontWeight: Fonts.semibold, height: 1.25, color: context.colors.text);

String _portions(int n) => n == 1 ? 'portion' : 'portions';

/// A weight in grams as the sheet says it: whole grams, or a tenth under 10.
String _grams(int mg) {
  if (mg >= 9950) return '${(mg / 1000).round()} g';
  final String tenths = (mg / 1000).toStringAsFixed(1);
  return '${tenths.endsWith('.0') ? tenths.substring(0, tenths.length - 2) : tenths} g';
}

String _time(BuildContext context, DateTime? at) => at == null ? '' : clockText(context, at);

class _Confirm extends StatefulWidget {
  const _Confirm({required this.feeder});

  final FeederController feeder;

  @override
  State<_Confirm> createState() => _ConfirmState();
}

class _ConfirmState extends State<_Confirm> {
  static const _most = 48;
  int _portions = 1;

  /// The portion size read as this sheet opened. Grams come only from it, never
  /// from an earlier reading another phone may since have changed.
  CalibrationInfo? _calibration;

  @override
  void initState() {
    super.initState();
    unawaited(_readPortion());
  }

  /// A feed never waits on this: until it answers, the sheet speaks in turns
  /// of the wheel.
  Future<void> _readPortion() async {
    final CalibrationInfo? read = await widget.feeder.loadCalibration();
    if (mounted) setState(() => _calibration = read);
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final int? portionMg = _calibration?.portionMg;
    final String? grams = portionMg == null ? null : _grams(portionMg * _portions);
    final String portion = portionMg == null
        ? "One portion is one turn of the feeder's wheel."
        : 'Each portion is about ${_grams(portionMg)}.';
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text('Feed ${widget.feeder.state.known.name} now', style: _title(context, size: 22)),
        const SizedBox(height: 20),
        Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            _Stepper(Glyph.minus, 'One fewer portion', _portions > 1 ? () => setState(() => _portions--) : null),
            const SizedBox(width: 28),
            SizedBox(
              width: 96,
              child: Column(
                children: [
                  Text(
                    '$_portions',
                    style: TextStyle(
                      fontSize: 56,
                      height: 1,
                      fontWeight: Fonts.display,
                      color: c.text,
                      fontFeatures: const [FontFeature.tabularFigures()],
                    ),
                  ),
                  const SizedBox(height: 6),
                  Text(
                    _portionsWord,
                    style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                  ),
                  if (grams != null)
                    Text(
                      '≈ $grams',
                      style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                    ),
                ],
              ),
            ),
            const SizedBox(width: 28),
            _Stepper(Glyph.add, 'One more portion', _portions < _most ? () => setState(() => _portions++) : null),
          ],
        ),
        const SizedBox(height: 20),
        Text(
          "$portion Today's meals still run.",
          textAlign: TextAlign.center,
          style: TextStyle(fontSize: Sizes.base, height: 1.5, color: c.textMuted),
        ),
        const SizedBox(height: 20),
        Row(
          children: [
            Expanded(
              child: OutlinedButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
            ),
            const SizedBox(width: Space.s3),
            // Twice Cancel's width, so "Feed 2 portions · 24 g" keeps to one line.
            Expanded(
              flex: 2,
              child: FilledButton(
                onPressed: widget.feeder.state.canFeed ? () => widget.feeder.feedNow(_portions) : null,
                child: Text('Feed $_portions $_portionsWord${grams == null ? '' : ' · $grams'}'),
              ),
            ),
          ],
        ),
      ],
    );
  }

  String get _portionsWord => _portions == 1 ? 'portion' : 'portions';
}

class _Progress extends StatelessWidget {
  const _Progress({required this.feeder});

  final FeederController feeder;

  @override
  Widget build(BuildContext context) {
    return switch (feeder.state.feed) {
      FeedSending() || FeedRunning() => Builder(
        builder: (context) {
          final ThemeTokens c = context.colors;
          final FeedState feed = feeder.state.feed;
          final running = feed is FeedRunning;
          return _Sheet(
            title: running ? 'Feeding $_name' : 'Sending…',
            children: [
              Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  ClipRRect(
                    borderRadius: BorderRadius.circular(Radii.pill),
                    child: LinearProgressIndicator(minHeight: 10, backgroundColor: c.sunken, color: c.food),
                  ),
                  const SizedBox(height: Space.s2),
                  _Muted('${feed.portions} ${_portions(feed.portions)}'),
                ],
              ),
              _Muted(
                "You can close this. The result shows on $_name's card and in its "
                'history.',
              ),
              if (running) ...[
                OutlinedButton(
                  style: OutlinedButton.styleFrom(foregroundColor: c.fault),
                  onPressed: feeder.cancelFeed,
                  child: const Text('Stop feeding'),
                ),
                if (feed.cancelUnconfirmed)
                  Text(
                    'The feeder did not confirm the stop; the feed may run to its '
                    'end.',
                    textAlign: TextAlign.center,
                    style: TextStyle(fontSize: Sizes.sm, color: c.warning),
                  )
                else if (feed.cancelAsked)
                  const _Muted('Asked the feeder to stop. Waiting for it to answer.', align: TextAlign.center)
                else
                  const _Muted('Stopping keeps what already dropped in the bowl.', align: TextAlign.center),
              ],
            ],
          );
        },
      ),
      FeedEnded(:final FeedResult result) when result.uncertain => Builder(
        builder: (context) {
          final ThemeTokens c = context.colors;
          return _Sheet(
            glyph: Glyph.warning,
            color: c.warning,
            title:
                'Fed between ${result.confirmed} and ${result.atMost} '
                '${_portions(result.atMost)}',
            children: [
              _Lead(_uncertainWhy(result.reason)),
              Container(
                padding: const EdgeInsets.only(left: Space.s3),
                decoration: BoxDecoration(
                  border: Border(left: BorderSide(color: c.warning, width: 3)),
                ),
                child: const _Muted(
                  "Wellfed never repeats a feed it can't confirm. Look at the "
                  'bowl before you feed again.',
                ),
              ),
              Row(
                children: [
                  Expanded(
                    child: OutlinedButton(
                      onPressed: () {
                        _close(context);
                        unawaited(context.push('/feeder/${feeder.state.known.identity}?tab=$historyTab'));
                      },
                      child: const Text('See history'),
                    ),
                  ),
                  const SizedBox(width: Space.s3),
                  Expanded(
                    child: FilledButton(onPressed: () => _close(context), child: const Text('Close')),
                  ),
                ],
              ),
            ],
          );
        },
      ),
      FeedEnded(:final FeedResult result) => Builder(
        builder: (context) {
          final ThemeTokens c = context.colors;
          final all = result.confirmed == result.requested;
          final FoodLevel? food = feeder.state.card?.food;
          return _Sheet(
            glyph: all ? Glyph.check : Glyph.warning,
            color: all ? c.ok : c.warning,
            title:
                'Fed ${result.confirmed} of ${result.requested} '
                '${_portions(result.requested)}',
            children: [
              if (!all) _Lead(_upper(result.words)),
              FactList(sunken: true, [
                if (feeder.state.feed case FeedEnded(endedAt: final DateTime endedAt))
                  ('Finished', Text(_time(context, endedAt))),
                if (food != null && food != FoodLevel.unread)
                  (
                    'Food level',
                    Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        FoodDot(present: food == FoodLevel.present),
                        const SizedBox(width: 6),
                        Text(food == FoodLevel.present ? 'Enough' : 'Low'),
                      ],
                    ),
                  ),
                ('Asked by', const Text('This phone')),
              ]),
              FilledButton(onPressed: () => _close(context), child: const Text('Close')),
            ],
          );
        },
      ),
      FeedRefused() || FeedNotSent() => Builder(
        builder: (context) {
          final ThemeTokens c = context.colors;
          final FeedState feed = feeder.state.feed;
          final String why = feed is FeedRefused
              ? _refusal(feed.refusal)
              : 'The connection dropped before the request went out. Try again '
                    'once the feeder is online.';
          return _Sheet(
            glyph: Glyph.failed,
            color: c.fault,
            title: "$_name didn't feed",
            children: [
              _Lead(why),
              const _Muted('Nothing was dispensed.'),
              Row(
                children: [
                  Expanded(
                    child: OutlinedButton(onPressed: () => _close(context), child: const Text('Close')),
                  ),
                  const SizedBox(width: Space.s3),
                  Expanded(
                    child: FilledButton(
                      // Nothing went out or the feeder said no: a new request is
                      // safe, from the confirm step.
                      onPressed: feeder.dismissFeed,
                      child: const Text('Start over'),
                    ),
                  ),
                ],
              ),
            ],
          );
        },
      ),
      FeedUnanswered() || FeedAdrift() => Builder(
        builder: (context) {
          final ThemeTokens c = context.colors;
          final FeedState feed = feeder.state.feed;
          final String sent = _time(context, feed.sentAt);
          final String lead = switch (feed) {
            FeedAdrift(by: FeedLoss.overdue) =>
              'The feed started${sent.isEmpty ? '' : ' at $sent'}, and the feeder '
                  "hasn't said how it ended, past the longest a feed can take. Its "
                  'history keeps the result if it recorded one.',
            FeedAdrift() =>
              'The feed started${sent.isEmpty ? '' : ' at $sent'}, then the '
                  'connection dropped. If the feeder stayed on, it finishes on '
                  'its own and keeps the result in its history.',
            _ =>
              'We sent the feed${sent.isEmpty ? '' : ' at $sent'}, then lost the '
                  'connection before the feeder answered.',
          };
          return _Sheet(
            glyph: Glyph.wifiOff,
            color: c.textMuted,
            title: 'Did $_name feed?',
            children: [
              _Lead(lead),
              const _Muted(
                "Wellfed won't send it again on its own. Look at the bowl before "
                'you feed again.',
              ),
              FilledButton(onPressed: () => _close(context), child: const Text('Close')),
            ],
          );
        },
      ),
      FeedIdle() => const SizedBox.shrink(),
    };
  }

  String get _name => feeder.state.known.name;

  void _close(BuildContext context) {
    feeder.dismissFeed();
    Navigator.pop(context);
  }

  static String _upper(String words) => words.isEmpty ? words : words[0].toUpperCase() + words.substring(1);

  static String _uncertainWhy(FeedReason reason) => switch (reason) {
    FeedReason.interrupted =>
      "The feeder restarted during the feed, so it can't say how much "
          'dropped.',
    FeedReason.power =>
      "The power went out during the feed, so the feeder can't say how much "
          'dropped.',
    FeedReason.sensor ||
    FeedReason.noFeedback ||
    FeedReason.feedbackStream => "The feeder lost track of its wheel, so it can't say how much dropped.",
    _ => 'The feeder stopped without recording how much dropped.',
  };

  static String _refusal(FeedRefusal? refusal) => switch (refusal) {
    FeedRefusal.forbidden => 'This phone may not feed this feeder.',
    FeedRefusal.busy => "It's busy saving a change. Try again in a moment.",
    FeedRefusal.fault => 'It stopped on a fault and feeds again after a restart.',
    FeedRefusal.portions => 'A feed is 1 to 48 portions.',
    FeedRefusal.notFound => 'That feed is not running.',
    FeedRefusal.unavailable => 'This firmware does not feed.',
    FeedRefusal.other || null => 'It refused the request.',
  };
}

class _Stepper extends StatelessWidget {
  const _Stepper(this.glyph, this.tooltip, this.onPressed);

  final Glyph glyph;
  final String tooltip;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return IconButton.outlined(
      tooltip: tooltip,
      constraints: const BoxConstraints.tightFor(width: 56, height: 56),
      style: IconButton.styleFrom(side: BorderSide(color: c.lineStrong)),
      onPressed: onPressed,
      icon: StrokeIcon(glyph, weight: 2, color: onPressed == null ? c.lineStrong : c.text),
    );
  }
}

/// One state of the sheet: an optional mark, a title and what it says.
class _Sheet extends StatelessWidget {
  const _Sheet({this.glyph, this.color, required this.title, required this.children});

  final Glyph? glyph;
  final Color? color;
  final String title;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Column(
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      if (glyph case final glyph?) ...[
        Align(
          alignment: Alignment.centerLeft,
          child: StrokeIcon(glyph, size: 40, color: color, weight: 2),
        ),
        const SizedBox(height: Space.s4),
      ],
      Text(title, style: _title(context)),
      for (final child in children) ...[const SizedBox(height: Space.s4), child],
    ],
  );
}

class _Lead extends StatelessWidget {
  const _Lead(this.text);

  final String text;

  @override
  Widget build(BuildContext context) =>
      Text(text, style: TextStyle(fontSize: 15, height: 1.5, color: context.colors.text));
}

class _Muted extends StatelessWidget {
  const _Muted(this.text, {this.align});

  final String text;
  final TextAlign? align;

  @override
  Widget build(BuildContext context) => Text(
    text,
    textAlign: align,
    style: TextStyle(fontSize: Sizes.base, height: 1.5, color: context.colors.textMuted),
  );
}
