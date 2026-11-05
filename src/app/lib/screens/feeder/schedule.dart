import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:wellfed/screens/feeder/meal_editor.dart';
import 'package:wellfed/screens/feeder/no_time.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/pending.dart';
import 'package:wellfed/widgets/when.dart';

/// The most meals a feeder keeps: what Rust reads back, all ten slots.
int mealsMost(PlansView plans) => plans.plans.length;

/// A meal's days in words.
String daysText(List<Weekday> days) {
  if (days.isEmpty) return 'once';
  if (days.length == 7) return 'every day';
  const List<Weekday> week = [Weekday.mon, Weekday.tue, Weekday.wed, Weekday.thu, Weekday.fri];
  const List<Weekday> weekend = [Weekday.sat, Weekday.sun];
  if (days.length == 5 && week.every(days.contains)) return 'Mon to Fri';
  if (days.length == 2 && weekend.every(days.contains)) return 'weekends';
  return [
    for (final day in Weekday.values)
      if (days.contains(day)) '${day.name[0].toUpperCase()}${day.name.substring(1)}',
  ].join(', ');
}

String _calls(int plays) => switch (plays) {
  1 => 'meal call once',
  2 => 'meal call twice',
  _ => 'meal call $plays times',
};

/// The feeder's meals (8.1), with its limit and hold (8.3). The table is the
/// feeder's: this shows the last it said, and each change goes back against
/// that reading. A meal's switch moves on tap and settles on the feeder's
/// answer.
class ScheduleTab extends StatefulWidget {
  const ScheduleTab({super.key, required this.feeder});

  final FeederController feeder;

  @override
  State<ScheduleTab> createState() => _ScheduleTabState();
}

class _ScheduleTabState extends State<ScheduleTab> {
  static const _thisTab = 1;

  TabController? _tabs;
  bool _opened = false;

  FeederController get feeder => widget.feeder;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final bool first = !_opened;
    _opened = true;
    _tabs?.removeListener(_tabChanged);
    _tabs = DefaultTabController.maybeOf(context)?..addListener(_tabChanged);
    if (first && (_tabs?.index ?? _thisTab) == _thisTab) _read();
  }

  @override
  void dispose() {
    _tabs?.removeListener(_tabChanged);
    super.dispose();
  }

  void _tabChanged() {
    final TabController? tabs = _tabs;
    if (tabs != null && tabs.index == _thisTab && !tabs.indexIsChanging) {
      _read();
    }
  }

  /// Coming here reads the plans again; the session puts them on the card. A
  /// read that fails leaves what the card last showed.
  void _read() {
    if (feeder.state.connection == Connection.online) {
      unawaited(feeder.ask(const FeederRequest.getPlans()));
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: feeder,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final Card? card = feeder.state.card;
      final PlansView? plans = card?.plans;
      if (plans == null) {
        return Center(
          child: Text(
            feeder.state.connection == Connection.online ? 'Reading the meals…' : "Can't read the meals right now.",
            style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
          ),
        );
      }

      final List<(int, PlanView)> meals = [
        for (final (slot, plan) in plans.plans.indexed)
          if (plan != null) (slot, plan),
      ]..sort((a, b) => a.$2.minute.compareTo(b.$2.minute));
      final int most = mealsMost(plans);
      final bool full = meals.length >= most;
      final online = feeder.state.connection == Connection.online;
      final NextMealView? next = card?.nextMeal;
      final muted = TextStyle(fontSize: Sizes.base, color: c.textMuted);

      return Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(
              child: ListView(
                children: [
                  if (online && !(card?.clockSet ?? true)) ...[
                    NoTimeNotice(feeder: feeder),
                    const SizedBox(height: Space.s3),
                  ],
                  Row(
                    mainAxisAlignment: MainAxisAlignment.spaceBetween,
                    children: [
                      Flexible(
                        flex: 2,
                        child: Text(
                          next == null
                              ? 'No meal is coming'
                              : 'Next: ${_day(next.atMs)} at '
                                    '${minuteText(context, next.minute)}',
                          style: TextStyle(fontSize: Sizes.base, fontWeight: Fonts.semibold, color: c.text),
                        ),
                      ),
                      const SizedBox(width: Space.s3),
                      Flexible(
                        child: Text(
                          full ? "$most of $most meals · the feeder's limit" : '${meals.length} of $most meals',
                          textAlign: TextAlign.end,
                          style: muted,
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: Space.s3),
                  if (meals.isNotEmpty)
                    Panel(
                      child: Column(
                        children: [
                          for (final (i, (slot, plan)) in meals.indexed)
                            _MealRow(
                              key: ValueKey(slot),
                              feeder: feeder,
                              plans: plans,
                              slot: slot,
                              plan: plan,
                              first: i == 0,
                              next: next?.slot == slot,
                              online: online,
                            ),
                        ],
                      ),
                    ),
                  const SizedBox(height: Space.s3),
                  Text(
                    'The feeder keeps these itself and feeds on time even when '
                    'no phone is around. A meal more than 15 minutes late is '
                    'missed and shows in History.',
                    style: muted.copyWith(height: 1.5),
                  ),
                ],
              ),
            ),
            const SizedBox(height: Space.s3),
            FilledButton(
              onPressed: online && !full ? () => unawaited(openMealEditor(context, feeder: feeder, held: plans)) : null,
              child: Text(full ? 'Add meal · delete one first' : 'Add meal'),
            ),
          ],
        ),
      );
    },
  );

  static String _day(int atMs) {
    final String day = dayText(DateTime.fromMillisecondsSinceEpoch(atMs));
    return day == 'Today' || day == 'Tomorrow' ? day.toLowerCase() : day;
  }
}

class _MealRow extends StatelessWidget {
  const _MealRow({
    super.key,
    required this.feeder,
    required this.plans,
    required this.slot,
    required this.plan,
    required this.first,
    required this.next,
    required this.online,
  });

  final FeederController feeder;
  final PlansView plans;
  final int slot;
  final PlanView plan;
  final bool first;

  /// The meal the feeder runs next.
  final bool next;
  final bool online;

  Future<FeederReply> _set(bool enabled) async {
    final FeederReply reply = await feeder.ask(
      FeederRequest.editPlans(
        plans: plans,
        edit: PlanEdit.enabled(slot: slot, enabled: enabled),
      ),
    );

    // Stale says the table moved: a read puts it on the card, and the owner
    // decides again from what it shows. The edit never goes again by itself.
    if (reply is FeederReply_Refused && reply.refusal is FeederRefusal_Stale) {
      unawaited(feeder.ask(const FeederRequest.getPlans()));
    }
    return reply;
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final String time = minuteText(context, plan.minute);
    // Off in words as well as on the switch, so the row says it without it.
    final String parts = [
      if (!plan.enabled) 'Off',
      portionsText(plan.portions),
      daysText(plan.days),
      if (plan.call > 0) _calls(plan.call),
    ].join(' · ');

    // On or off as asked from the tap until the card agrees; a refusal moves
    // it back. A node of its own: with no tap to tell them apart, as offline,
    // every meal on the panel would merge into one.
    return Pending<bool>(
      card: plan.enabled,
      generation: plans.generation,
      builder: (context, enabled, write) => Semantics(
        container: true,
        child: InkWell(
          onTap: online ? () => unawaited(openMealEditor(context, feeder: feeder, held: plans, slot: slot)) : null,
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: Space.s4, vertical: Space.s2),
            decoration: BoxDecoration(
              border: first ? null : Border(top: BorderSide(color: c.line)),
            ),
            child: Row(
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Text(
                            time,
                            style: TextStyle(
                              fontSize: Sizes.md,
                              fontWeight: Fonts.semibold,
                              color: enabled ? c.text : c.textMuted,
                            ),
                          ),
                          if (next) ...[
                            const SizedBox(width: Space.s3),
                            Flexible(child: StatusChip('Next', color: c.accent)),
                          ],
                        ],
                      ),
                      Text(
                        parts,
                        style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                      ),
                    ],
                  ),
                ),
                Semantics(
                  container: true,
                  label: '$time ${enabled ? 'on' : 'off'}',
                  child: Switch(
                    value: enabled,
                    onChanged: online ? (next) => unawaited(write.ask(next, () => _set(next))) : null,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// A row that says what the meal call is, for the editor and the settings.
class CallLink extends StatelessWidget {
  const CallLink({super.key, required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return InkWell(
      onTap: onTap,
      // Android's 48dp target, however the text sizes.
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 48),
        child: Row(
          children: [
            Expanded(
              child: Text(
                'Your recording, before the food drops',
                style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
              ),
            ),
            StrokeIcon(Glyph.chevron, size: 18, color: c.textMuted),
          ],
        ),
      ),
    );
  }
}
