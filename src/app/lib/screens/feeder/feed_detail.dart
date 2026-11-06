import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/screens/feeder/history.dart' show whoText;
import 'package:wellfed/screens/feeder/motor_trace.dart';
import 'package:wellfed/screens/feeder/today_faults.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/when.dart';

/// One feed, in detail (9.2), as its own page. A desktop's History shows
/// [FeedDetailBody] beside the list instead.
class FeedDetail extends StatelessWidget {
  const FeedDetail({super.key, required this.feeder, required this.result});

  final FeederController feeder;
  final FeedResult result;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
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
          feedWhen(context, result),
          style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
        ),
      ),
      body: FeedDetailBody(feeder: feeder, result: result),
    );
  }
}

/// When a feed ran, as its detail heads it.
String feedWhen(BuildContext context, FeedResult result) {
  final PlatformInt64? at = result.atMs;
  return at == null
      ? 'Time unknown'
      : '${dayText(DateTime.fromMillisecondsSinceEpoch(at))}, '
            '${clockTextMs(context, at)}';
}

/// What one feed did. An uncertain feed stays a range.
class FeedDetailBody extends StatelessWidget {
  const FeedDetailBody({super.key, required this.feeder, required this.result, this.heading});

  final FeederController feeder;
  final FeedResult result;

  /// A heading over it, where no app bar names the feed.
  final Widget? heading;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final bool all = result.outcome == FeedOutcome.done && result.confirmed == result.requested;
    final title = result.uncertain
        ? 'Fed between ${result.confirmed} and ${result.atMost} portions'
        : 'Fed ${deliveredText(result)} ${portionWord(result.requested)}';
    final detail = result.uncertain
        ? '${result.requested} asked for · '
              '${result.confirmed == 0 ? 'none' : result.confirmed} confirmed '
              '· each could have dropped'
        : '${result.requested} asked for';

    return ListView(
      padding: const EdgeInsets.all(Space.s4),
      children: [
        if (heading case final heading?) ...[heading, const SizedBox(height: Space.s4)],
        Align(
          alignment: Alignment.centerLeft,
          child: StrokeIcon(all ? Glyph.check : Glyph.warning, size: 40, weight: 2, color: all ? c.ok : c.warning),
        ),
        const SizedBox(height: Space.s3),
        Text(
          title,
          style: TextStyle(fontSize: Sizes.lg, fontWeight: Fonts.semibold, color: c.text),
        ),
        const SizedBox(height: Space.s1),
        Text(
          detail,
          style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
        ),
        const SizedBox(height: Space.s4),
        FactList([
          if (!all)
            (
              result.uncertain ? "Why it's unsure" : 'Why it stopped',
              Flexible(child: Text(faultTitle(context, result.reason, null), textAlign: TextAlign.end)),
            ),
          ('Started by', Text(whoText(feeder, result.by))),
          ('Food level', Text(_hopper(result.hopper))),
          ('Feed number', Text('${result.feed}')),
        ]),
        if (result.uncertain) ...[
          const SizedBox(height: Space.s4),
          Text(
            'When a feed is cut short, the feeder records the range it '
            "might have dropped and never runs it again, so your pet can't "
            'be double-fed.',
            style: TextStyle(fontSize: Sizes.base, height: 1.5, color: c.textMuted),
          ),
        ],
        const SizedBox(height: Space.s4),
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton(
            onPressed: () => unawaited(
              Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => MotorTraceScreen(feeder: feeder, feed: result.feed),
                ),
              ),
            ),
            child: const Text('Motor trace for this feed'),
          ),
        ),
      ],
    );
  }

  static String _hopper(FoodLevel level) => switch (level) {
    FoodLevel.present => 'Enough',
    FoodLevel.low => 'Low',
    FoodLevel.unread => 'Not checked',
  };
}
