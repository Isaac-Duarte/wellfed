import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/widgets/replies.dart';
import 'package:wellfed/widgets/when.dart';

/// Skip one of today's meals, or undo its skip: a meal row's button. Which
/// meal, and whether either is on offer, is the card's: the bridge reckons
/// it from the plans on the feeder's clock, and the request names the meal
/// by the instant it gave. The feeder checks it again, so a tap that came
/// too late is refused, and nothing is sent twice.
///
/// A change the feeder took shows on the card only once its event has the
/// session read the plans again, and until then the row still offers what
/// it offered. So after a `Done` the button stays off until the card shows
/// the meal otherwise, or the session drops: a second tap would be refused,
/// and its refusal could not say whether the meal was back on.
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

  /// How the meal stood when the feeder took a change of it: the button
  /// stays off while the card still shows it so.
  (api.MealStanding, api.MealAction)? _took;

  /// The slot, the time and the minute are read together at the tap, so a
  /// card that changes while the dialog is open can't mix two meals.
  Future<void> _skip(int slot, PlatformInt64 dueMs, int minute) async {
    final bool? sure = await showDialog<bool>(
      context: context,
      builder: (context) => SkipMealDialog(minute: minute),
    );
    if (sure != true || !mounted) return;
    await _ask(FeederRequest.skipMeal(slot: slot, dueMs: dueMs), _Change.skip);
  }

  Future<void> _ask(FeederRequest request, _Change change) async {
    final (api.MealStanding, api.MealAction) asked = (widget.meal.standing, widget.action);
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(request);
    if (!mounted) return;
    setState(() {
      _asking = false;
      if (reply is FeederReply_Done) _took = asked;
    });

    final String? problem = skipProblem(reply, undo: change == _Change.undo);
    if (problem == null) return;
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(problem)));
  }

  @override
  Widget build(BuildContext context) {
    // The meal's standing is the card's: only this session's card can say.
    final bool online = widget.feeder.state.cardIsCurrent;
    // The hold ends once the card shows the meal otherwise, or the session
    // drops: a new session reads the card afresh.
    if (!online || _took != (widget.meal.standing, widget.action)) _took = null;
    final bool can = online && !_asking && _took == null;
    final int slot = widget.meal.slot;
    final int minute = widget.meal.minute;
    final String at = minuteText(context, minute);
    return switch (widget.action) {
      api.MealAction_Skip(:final PlatformInt64 dueMs) => TextButton(
        onPressed: can ? () => unawaited(_skip(slot, dueMs, minute)) : null,
        child: Text('Skip', semanticsLabel: 'Skip the $at meal'),
      ),
      api.MealAction_Undo(:final PlatformInt64 dueMs) => TextButton(
        onPressed: can ? () => unawaited(_ask(FeederRequest.unskipMeal(slot: slot, dueMs: dueMs), _Change.undo)) : null,
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
        "It won't feed, remind or play its call. Tomorrow's meal runs as usual. "
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
/// `invalid` for a meal that is no longer as the card showed it, with no
/// word of which way: due already, skipped or brought back already (from
/// here or another phone), or moved by a plan change. So the words say
/// what may be true, not which.
String? skipProblem(FeederReply reply, {required bool undo}) => switch (reply) {
  FeederReply_Refused(refusal: FeederRefusal_Invalid()) when undo =>
    "Couldn't undo. The meal may already be back on, or its time has come: look at Today again.",
  FeederReply_Refused(refusal: FeederRefusal_Invalid()) =>
    "Couldn't skip. The meal may already be skipped, due, or no longer next: look at Today again.",
  FeederReply_Refused(refusal: FeederRefusal_Busy()) =>
    'The feeder is busy with a feed or a change to the meals. Try again in a moment.',
  FeederReply_Refused(refusal: FeederRefusal_NotFound()) => "That meal isn't on the schedule any more.",
  _ => replyProblem(reply),
};
