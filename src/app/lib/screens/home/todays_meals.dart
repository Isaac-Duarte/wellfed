import 'dart:async';

import 'package:clock/clock.dart';
import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/when.dart';

/// What today's meals come to (5.1), below the only card.
class TodaysMeals extends StatelessWidget {
  const TodaysMeals({super.key, required this.controller});

  final FeederController controller;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final List<api.MealView> meals = controller.state.card?.meals ?? const <api.MealView>[];
      if (meals.isEmpty) return const SizedBox.shrink();
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const SizedBox(height: Space.s4),
          Row(
            children: [
              Expanded(
                child: Text(
                  "Today's meals",
                  style: TextStyle(fontSize: Sizes.md, fontWeight: Fonts.semibold, color: c.text),
                ),
              ),
              TextButton(
                onPressed: () => unawaited(context.push('/feeder/${controller.state.known.identity}?tab=$scheduleTab')),
                child: const Text('Schedule'),
              ),
            ],
          ),
          Panel(
            child: Column(children: [for (final (i, meal) in meals.indexed) _MealRow(meal, first: i == 0)]),
          ),
        ],
      );
    },
  );
}

class _MealRow extends StatelessWidget {
  const _MealRow(this.meal, {required this.first});

  final api.MealView meal;
  final bool first;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final String portions = portionsText(meal.portions);
    final what = meal.call > 0 ? '$portions · meal call' : portions;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: Space.s4, vertical: 12),
      decoration: BoxDecoration(
        border: first ? null : Border(top: BorderSide(color: c.line)),
      ),
      // The time sits above what the meal gives, not in a fixed column beside
      // it: "12:00 PM" is wider than "7:00 AM", and wider still in large text.
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  minuteText(context, meal.minute),
                  style: TextStyle(fontSize: 15, fontWeight: Fonts.semibold, color: c.text),
                ),
                Text(
                  what,
                  style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                ),
              ],
            ),
          ),
          const SizedBox(width: Space.s3),
          _Standing(meal),
        ],
      ),
    );
  }
}

/// How one of today's meals stands.
class _Standing extends StatelessWidget {
  const _Standing(this.meal);

  final api.MealView meal;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return switch (meal.standing) {
      api.MealStanding_Fed(:final api.FeedResult result) when result.uncertain => _Words('Uncertain', c.warning),
      api.MealStanding_Fed(:final api.FeedResult result) when result.confirmed == result.requested => Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          StrokeIcon(Glyph.check, size: 16, color: c.ok, weight: 2),
          const SizedBox(width: Space.s1),
          _Words('Fed', c.ok),
        ],
      ),
      api.MealStanding_Fed(:final api.FeedResult result) => _Words(
        'Fed ${result.confirmed} of ${result.requested}',
        c.warning,
      ),
      api.MealStanding_Passed() => _Words('No feed recorded', c.textMuted),
      api.MealStanding_Next(:final PlatformInt64 atMs) => _Words(untilText(clock.now(), atMs), c.accent),
      api.MealStanding_Later() => _Words('Later', c.textMuted),
    };
  }
}

class _Words extends StatelessWidget {
  const _Words(this.text, this.color);

  final String text;
  final Color color;

  @override
  Widget build(BuildContext context) => Text(
    text,
    style: TextStyle(fontSize: Sizes.base, fontWeight: Fonts.medium, color: color),
  );
}
