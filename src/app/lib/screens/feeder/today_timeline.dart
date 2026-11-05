import 'dart:async';
import 'dart:math' as math;

import 'package:clock/clock.dart';
import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/screens/feeder/feed_detail.dart';
import 'package:wellfed/screens/feeder/history.dart';
import 'package:wellfed/screens/feeder/meal_skip.dart';
import 'package:wellfed/screens/feeder/portion.dart' show gramsText;
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/src/rust/api/session.dart' show FedBy, FedBy_Client, FedBy_Plan, FeedResult;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/when.dart';

/// How today went, the question the tab is opened for: what the cat has been
/// given so far, then every meal and every feed asked for by hand, in time
/// order.
///
/// The meals and how each stands come from the card; a feed asked for by hand
/// and the reason a meal was skipped come from the history the session keeps,
/// so a history not read yet shows the meals alone.
class TodayTimeline extends StatelessWidget {
  const TodayTimeline({super.key, required this.feeder, required this.card});

  final FeederController feeder;
  final api.Card card;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final List<_Moment> moments = _moments(feeder, card);
    final List<FeedResult> given = [
      for (final moment in moments)
        if (moment.fed case final FeedResult fed) fed,
    ];
    final int portions = given.fold(0, (sum, fed) => sum + fed.confirmed);
    // An unsure feed gave between what it confirmed and what it may have
    // dropped: the day is a range then, never one number.
    final int most = given.fold(0, (sum, fed) => sum + (fed.uncertain ? fed.atMost : fed.confirmed));
    // Grams once a screen that needs the portion size has read it; Today
    // asks the feeder nothing of its own.
    final int? mg = feeder.state.calibration?.portionMg;
    final TabController? tabs = DefaultTabController.maybeOf(context);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const Padding(
          padding: EdgeInsets.only(left: Space.s1, bottom: Space.s1),
          child: SectionLabel('Fed today'),
        ),
        Padding(
          padding: const EdgeInsets.only(left: Space.s1, bottom: Space.s2),
          child: Text.rich(
            TextSpan(
              children: [
                TextSpan(
                  text: portions == 0 && given.isEmpty
                      ? 'Nothing yet'
                      : most > portions
                      ? '$portions to ${portionsText(most)}'
                      : portionsText(portions),
                  style: TextStyle(fontSize: 28, fontWeight: Fonts.semibold, color: c.text),
                ),
                if (mg != null && portions > 0)
                  TextSpan(
                    text: '   ≈ ${gramsText(mg * portions)} g',
                    style: TextStyle(fontSize: Sizes.md, color: c.textMuted),
                  ),
              ],
            ),
          ),
        ),
        // Nothing today: when the cat last had something, as the card says.
        if (given.isEmpty)
          if (card.lastFed case final FeedResult last?)
            Padding(
              padding: const EdgeInsets.only(left: Space.s1, bottom: Space.s3),
              child: Text(switch (last.atMs) {
                final int at => 'Last fed ${momentTextMs(context, at)} · ${fedAmountText(last)}',
                null => 'Last fed · ${fedAmountText(last)}',
              }, style: TextStyle(fontSize: Sizes.base, color: c.textMuted)),
            ),
        if (moments.isEmpty)
          Panel(
            padding: const EdgeInsets.fromLTRB(Space.s4, Space.s2, Space.s2, Space.s2),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    'No meals today',
                    style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                  ),
                ),
                // Only inside the feeder's tabs, where the schedule is one
                // tap away.
                if (tabs != null)
                  TextButton(onPressed: () => tabs.animateTo(scheduleTab), child: const Text('Add a meal')),
              ],
            ),
          )
        else
          _Rows(feeder: feeder, moments: moments),
      ],
    );
  }
}

/// One line of the day: a meal in whatever standing, or a feed by hand.
class _Moment {
  const _Moment({required this.minute, required this.standing, this.meal, this.fed, this.missed});

  /// Minutes into the day, for the order and the time shown.
  final int minute;
  final api.MealStanding standing;
  final api.MealView? meal;

  /// What the feed gave, when there was one.
  final FeedResult? fed;

  /// Why a passed meal didn't run, when the history says.
  final MissedReason? missed;
}

List<_Moment> _moments(FeederController feeder, api.Card card) {
  final DateTime now = clock.now();
  bool today(int ms) => DateUtils.isSameDay(DateTime.fromMillisecondsSinceEpoch(ms), now);
  int minuteOf(int ms) {
    final at = DateTime.fromMillisecondsSinceEpoch(ms);
    return at.hour * 60 + at.minute;
  }

  final missed = <int, MissedReason>{};
  final byHand = <_Moment>[];
  for (final api.HistoryEntry entry in feeder.state.history) {
    switch (entry.body) {
      case api.HistoryBody_Missed(:final PlatformInt64 dueMs, :final MissedReason reason) when today(dueMs):
        missed[minuteOf(dueMs)] = reason;
      case api.HistoryBody_Fed(:final FeedResult result) when result.by is! FedBy_Plan:
        // A meal's own feed is on its meal's line already.
        if (result.atMs ?? entry.atMs case final int at when today(at)) {
          byHand.add(
            _Moment(
              minute: minuteOf(at),
              standing: api.MealStanding.fed(result: result),
              fed: result,
            ),
          );
        }
      default:
    }
  }

  // The card's last feed, when it ran today and the history hasn't brought
  // it yet: the card is read first.
  if (card.lastFed case final FeedResult last? when last.by is! FedBy_Plan) {
    if (last.atMs case final int at when today(at) && byHand.every((moment) => moment.fed?.feed != last.feed)) {
      byHand.add(
        _Moment(
          minute: minuteOf(at),
          standing: api.MealStanding.fed(result: last),
          fed: last,
        ),
      );
    }
  }

  return [
    for (final api.MealView meal in card.meals)
      _Moment(
        minute: meal.minute,
        standing: meal.standing,
        meal: meal,
        fed: switch (meal.standing) {
          api.MealStanding_Fed(:final FeedResult result) => result,
          _ => null,
        },
        missed: missed[meal.minute],
      ),
    ...byHand,
  ]..sort((a, b) => a.minute.compareTo(b.minute));
}

class _Rows extends StatelessWidget {
  const _Rows({required this.feeder, required this.moments});

  final FeederController feeder;
  final List<_Moment> moments;

  @override
  Widget build(BuildContext context) {
    final TextStyle time = _timeStyle(context);
    // One column for the times, as wide as the widest: "12:00 PM" is wider
    // than "7:00 AM", and the titles should line up.
    final TextScaler scaler = MediaQuery.textScalerOf(context);
    double widest = 0;
    for (final _Moment moment in moments) {
      final TextPainter painter = TextPainter(
        text: TextSpan(text: minuteText(context, moment.minute), style: time),
        textDirection: Directionality.of(context),
        textScaler: scaler,
      )..layout();
      widest = math.max(widest, painter.width);
      painter.dispose();
    }

    return Panel(
      child: Column(
        children: [
          for (final (i, moment) in moments.indexed)
            _Row(feeder: feeder, moment: moment, first: i == 0, timeWidth: widest.ceilToDouble()),
        ],
      ),
    );
  }
}

TextStyle _timeStyle(BuildContext context) => TextStyle(
  fontSize: Sizes.base,
  fontWeight: Fonts.medium,
  color: context.colors.textMuted,
  fontFeatures: const [FontFeature.tabularFigures()],
);

class _Row extends StatelessWidget {
  const _Row({required this.feeder, required this.moment, required this.first, required this.timeWidth});

  final FeederController feeder;
  final _Moment moment;
  final bool first;
  final double timeWidth;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final (Glyph glyph, Color color, String title, String note, bool loud) = _words(context, c);
    final FeedResult? fed = moment.fed;
    final bool next = moment.standing is api.MealStanding_Next;

    return InkWell(
      onTap: fed == null
          ? null
          : () => unawaited(
              Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => FeedDetail(feeder: feeder, result: fed),
                ),
              ),
            ),
      child: Container(
        constraints: const BoxConstraints(minHeight: 56),
        padding: const EdgeInsets.symmetric(horizontal: Space.s3, vertical: Space.s3),
        decoration: BoxDecoration(
          color: next ? c.accent.withValues(alpha: 0.08) : null,
          border: first ? null : Border(top: BorderSide(color: c.line)),
        ),
        child: Row(
          children: [
            SizedBox(
              width: timeWidth,
              child: Text(minuteText(context, moment.minute), style: _timeStyle(context)),
            ),
            const SizedBox(width: Space.s3),
            Container(
              width: 28,
              height: 28,
              alignment: Alignment.center,
              decoration: loud
                  ? BoxDecoration(color: color.withValues(alpha: 0.16), shape: BoxShape.circle)
                  : BoxDecoration(
                      shape: BoxShape.circle,
                      border: Border.all(color: c.line),
                    ),
              child: StrokeIcon(glyph, size: 14, color: color, weight: 2),
            ),
            const SizedBox(width: Space.s3),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    title,
                    style: TextStyle(
                      fontSize: Sizes.base,
                      fontWeight: loud ? Fonts.semibold : Fonts.medium,
                      color: loud ? c.text : c.textMuted,
                    ),
                  ),
                  if (note.isNotEmpty)
                    Text(
                      note,
                      style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                    ),
                ],
              ),
            ),
            if (moment.meal case api.MealView(:final api.MealAction action?) && final api.MealView meal)
              MealSkipButton(feeder: feeder, meal: meal, action: action),
            if (fed != null) StrokeIcon(Glyph.chevron, size: 16, color: c.textMuted),
          ],
        ),
      ),
    );
  }

  /// The badge, its color, the line and the note under it, and whether the
  /// line is one the owner should see first.
  (Glyph, Color, String, String, bool) _words(BuildContext context, ThemeTokens c) {
    final api.MealView? meal = moment.meal;
    final String call = (meal?.call ?? 0) > 0 ? 'meal call' : '';
    String join(List<String> parts) => parts.where((part) => part.isNotEmpty).join(' · ');

    return switch ((moment.standing, moment.fed)) {
      (_, final FeedResult fed?) when fed.uncertain => (
        Glyph.warning,
        c.warning,
        'Fed between ${fed.confirmed} and ${fed.atMost}',
        join([_who(fed), 'not repeated']),
        true,
      ),
      (_, final FeedResult fed?) when fed.confirmed != fed.requested => (
        Glyph.warning,
        c.warning,
        'Fed ${fedAmountText(fed)}',
        _who(fed),
        true,
      ),
      (_, final FeedResult fed?) => (Glyph.check, c.ok, 'Fed ${fedAmountText(fed)}', _who(fed), true),
      (api.MealStanding_Passed(), _) => (
        Glyph.warning,
        c.warning,
        'Skipped · ${portionsText(meal?.portions ?? 0)}',
        switch (moment.missed) {
          final MissedReason reason? => missedText(reason),
          null => 'No feed recorded',
        },
        true,
      ),
      // Skipped on request: quiet, and not a problem, unlike a meal the
      // feeder couldn't run.
      (api.MealStanding_Skipped() || api.MealStanding_SkipKept(), _) => (
        Glyph.minus,
        c.textMuted,
        'Skipped · ${portionsText(meal?.portions ?? 0)}',
        'Skipped on request',
        false,
      ),
      (api.MealStanding_Next(:final PlatformInt64 atMs), _) => (
        Glyph.clock,
        c.accent,
        'Next · ${untilText(clock.now(), atMs)}',
        join([portionsText(meal?.portions ?? 0), call]),
        true,
      ),
      _ => (Glyph.clock, c.textMuted, portionsText(meal?.portions ?? 0), join(['Later', call]), false),
    };
  }

  /// Who asked: a meal says nothing more, its line is the meal.
  String _who(FeedResult fed) => switch (fed.by) {
    FedBy_Plan() => '',
    FedBy_Client() => 'Feed now · ${whoText(feeder, fed.by)}',
    final FedBy by => whoText(feeder, by),
  };
}
