import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/widgets/replies.dart';
import 'package:wellfed/widgets/when.dart';

/// Skip one of today's meals, or undo its skip: a meal row's button. Which
/// meal, and whether either is on offer, is the card's: the bridge reckons
/// it from the plans on the feeder's clock, and the request names the meal
/// by the instant it gave. The feeder checks it again, so a tap that came
/// too late is refused, and nothing is sent twice.
class MealSkipButton extends StatefulWidget {
  const MealSkipButton({super.key, required this.feeder, required this.meal, required this.action});

  final FeederController feeder;
  final api.MealView meal;
  final api.MealAction action;

  @override
  State<MealSkipButton> createState() => _MealSkipButtonState();
}

class _MealSkipButtonState extends State<MealSkipButton> {
  bool _asking = false;

  Future<void> _skip(PlatformInt64 dueMs) async {
    final bool? sure = await showDialog<bool>(
      context: context,
      builder: (context) => SkipMealDialog(minute: widget.meal.minute),
    );
    if (sure != true || !mounted) return;
    await _ask(FeederRequest.skipMeal(slot: widget.meal.slot, dueMs: dueMs), _Change.skip);
  }

  Future<void> _ask(FeederRequest request, _Change change) async {
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(request);
    if (!mounted) return;
    setState(() => _asking = false);

    final String? problem = skipProblem(reply, undo: change == _Change.undo);
    if (problem == null) return;
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(problem)));
  }

  @override
  Widget build(BuildContext context) {
    final bool online = widget.feeder.state.connection == Connection.online;
    final bool can = online && !_asking;
    final String at = minuteText(context, widget.meal.minute);
    return switch (widget.action) {
      api.MealAction_Skip(:final PlatformInt64 dueMs) => TextButton(
        onPressed: can ? () => unawaited(_skip(dueMs)) : null,
        child: Text('Skip', semanticsLabel: 'Skip the $at meal'),
      ),
      api.MealAction_Undo(:final PlatformInt64 dueMs) => TextButton(
        onPressed: can
            ? () => unawaited(_ask(FeederRequest.unskipMeal(slot: widget.meal.slot, dueMs: dueMs), _Change.undo))
            : null,
        child: Text('Undo', semanticsLabel: 'Undo skipping the $at meal'),
      ),
    };
  }
}

enum _Change { skip, undo }

/// Asks before a skip, which a meal's time makes final.
class SkipMealDialog extends StatelessWidget {
  const SkipMealDialog({super.key, required this.minute});

  /// The meal's minute of the feeder's day.
  final int minute;

  @override
  Widget build(BuildContext context) {
    final String at = minuteText(context, minute);
    return AlertDialog(
      title: Text('Skip the $at meal today?'),
      content: Text(
        "It won't feed, remind or play its call. Tomorrow's runs as usual. "
        'You can undo this until $at.',
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Keep it')),
        FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Skip meal')),
      ],
    );
  }
}

/// Why a skip or its undo came to nothing, if it did. The feeder refuses
/// `invalid` when the meal is no longer the one the card showed: due
/// already, skipped already, or moved.
String? skipProblem(FeederReply reply, {required bool undo}) => switch (reply) {
  FeederReply_Refused(refusal: FeederRefusal_Invalid()) when undo =>
    "That meal is already due, so it stays skipped. A meal can't be brought back once its time comes.",
  FeederReply_Refused(refusal: FeederRefusal_Invalid()) =>
    "That meal can't be skipped now: it's due, already skipped, or no longer next. Look again and decide again.",
  FeederReply_Refused(refusal: FeederRefusal_Busy()) =>
    'The feeder is busy with a feed or a change to the meals. Try again in a moment.',
  FeederReply_Refused(refusal: FeederRefusal_NotFound()) => "That meal isn't on the schedule any more.",
  _ => replyProblem(reply),
};
