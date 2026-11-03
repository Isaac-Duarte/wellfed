import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/feeder/meal_call.dart';
import 'package:wellfed/screens/feeder/meal_choices.dart';
import 'package:wellfed/screens/feeder/schedule.dart' show CallLink;
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/replies.dart';
import 'package:wellfed/widgets/when.dart';

/// Open the editor over the plans the owner sees; `slot` for an existing
/// meal, none for a new one.
Future<void> openMealEditor(
  BuildContext context, {
  required FeederController feeder,
  required PlansView held,
  int? slot,
}) => Navigator.of(context).push(
  MaterialPageRoute<void>(
    fullscreenDialog: true,
    builder: (_) => MealEditor(feeder: feeder, held: held, slot: slot),
  ),
);

const List<Weekday> _everyDay = Weekday.values;
const List<Weekday> _weekdays = [Weekday.mon, Weekday.tue, Weekday.wed, Weekday.thu, Weekday.fri];
const List<Weekday> _weekend = [Weekday.sat, Weekday.sun];

/// Edit one meal (8.2). A save goes back against the table this opened on, so a
/// change meanwhile (another client's, or a one-off meal disabling itself as it
/// runs) is refused and asked about (8.4), never overwritten unseen.
class MealEditor extends StatefulWidget {
  const MealEditor({super.key, required this.feeder, required this.held, this.slot});

  final FeederController feeder;
  final PlansView held;
  final int? slot;

  @override
  State<MealEditor> createState() => _MealEditorState();
}

class _MealEditorState extends State<MealEditor> {
  late int _minute;
  late int _portions;
  late Set<Weekday> _days;
  late int _call;
  late bool _enabled;
  bool _asking = false;

  /// What the next save goes against.
  late PlansView _held = widget.held;

  PlanView? get _original {
    final int? slot = widget.slot;
    return slot == null ? null : widget.held.plans[slot];
  }

  @override
  void initState() {
    super.initState();
    final PlanView? plan = _original;
    _minute = plan?.minute ?? 8 * 60;
    _portions = plan?.portions ?? 1;
    _days = (plan?.days ?? _everyDay).toSet();
    _call = plan?.call ?? 0;
    _enabled = plan?.enabled ?? true;
  }

  PlanView get _plan => PlanView(
    minute: _minute,
    portions: _portions,
    days: [
      for (final day in Weekday.values)
        if (_days.contains(day)) day,
    ],
    enabled: _enabled,
    call: _call,
  );

  PlanEdit _edit(PlansView against) {
    final int? slot = widget.slot;
    final PlanView plan = _plan;
    // The slot emptied meanwhile: "Save mine" brings the meal back.
    return slot == null || against.plans[slot] == null
        ? PlanEdit.add(plan: plan)
        : PlanEdit.change(
            slot: slot,
            minute: plan.minute,
            portions: plan.portions,
            days: plan.days,
            call: plan.call,
            enabled: plan.enabled,
          );
  }

  Future<void> _save() async {
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(FeederRequest.editPlans(plans: _held, edit: _edit(_held)));
    if (!mounted) return;
    setState(() => _asking = false);

    if (reply is FeederReply_Refused && reply.refusal is FeederRefusal_Stale) {
      await _changedMeanwhile(
        wrote: "Your change wasn't saved, so nothing was overwritten.",
        yours: (context) => [('Your version', _line(context, _plan))],
        keep: "Keep what's there",
        go: 'Save mine over it',
        again: _save,
      );
    } else if (showProblem(context, reply)) {
      Navigator.pop(context);
    }
  }

  /// The meals moved first (8.4): show them, and let the owner choose. Going on
  /// is a new write of their choosing, against what the feeder now holds; nothing
  /// here sends the edit by itself.
  Future<void> _changedMeanwhile({
    required String wrote,
    required List<(String, String)> Function(BuildContext) yours,
    required String keep,
    required String go,
    required Future<void> Function() again,
  }) async {
    final FeederReply reply = await widget.feeder.ask(const FeederRequest.getPlans());
    if (!mounted) return;
    if (reply is! FeederReply_Plans) {
      showProblem(context, reply);
      return;
    }

    final int? slot = widget.slot;
    final PlanView? theirs = slot == null ? null : reply.plans.plans[slot];
    final bool? mine = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        scrollable: true,
        title: const Text('The meals changed while you were editing'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('Someone else changed them, or a one-off meal ran. $wrote'),
            const SizedBox(height: Space.s3),
            FactList(sunken: true, [
              if (slot != null)
                (
                  'Now on the feeder',
                  Flexible(child: Text(theirs == null ? 'Deleted' : _line(context, theirs), textAlign: TextAlign.end)),
                ),
              for (final (name, line) in yours(context)) (name, Flexible(child: Text(line, textAlign: TextAlign.end))),
            ]),
          ],
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: Text(keep)),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: Text(go)),
        ],
      ),
    );
    if (!mounted) return;

    if (mine == true) {
      _held = reply.plans;
      await again();
    } else if (mine == false) {
      Navigator.pop(context);
    }
  }

  static String _line(BuildContext context, PlanView plan) =>
      '${minuteText(context, plan.minute)} · ${portionsText(plan.portions)}';

  /// Ask first: one tap would take the meal off the schedule. A delete chosen
  /// again after a stale refusal has been asked already, so it skips this.
  Future<void> _confirmDelete() async {
    final String time = minuteText(context, _original?.minute ?? _minute);
    final bool? sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Delete this meal?'),
        content: Text('The $time meal comes off the schedule.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Delete')),
        ],
      ),
    );
    if (sure == true && mounted) await _delete();
  }

  Future<void> _delete() async {
    final int? slot = widget.slot;
    if (slot == null) return;
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(
      FeederRequest.editPlans(
        plans: _held,
        edit: PlanEdit.delete(slot: slot),
      ),
    );
    if (!mounted) return;
    setState(() => _asking = false);

    if (reply is FeederReply_Refused && reply.refusal is FeederRefusal_Stale) {
      await _changedMeanwhile(
        wrote: "This meal wasn't deleted.",
        yours: (_) => const [],
        keep: 'Keep it',
        go: 'Delete it',
        again: _delete,
      );
    } else if (reply is FeederReply_NotAsked && reply.reason == NotAsked.emptySlot) {
      // Gone already, which is what the owner asked for.
      Navigator.pop(context);
    } else if (showProblem(context, reply)) {
      Navigator.pop(context);
    }
  }

  Future<void> _pickTime() async {
    final TimeOfDay? picked = await showTimePicker(
      context: context,
      initialTime: TimeOfDay(hour: _minute ~/ 60, minute: _minute % 60),
    );
    if (picked != null) {
      setState(() => _minute = picked.hour * 60 + picked.minute);
    }
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final String time = minuteText(context, _minute);
    final label = TextStyle(fontSize: Sizes.base, fontWeight: Fonts.semibold, color: c.text);
    final muted = TextStyle(fontSize: Sizes.sm, color: c.textMuted);
    bool same(List<Weekday> days) => _days.length == days.length && days.every(_days.contains);

    return Scaffold(
      appBar: AppBar(
        toolbarHeight: 64,
        automaticallyImplyLeading: false,
        titleSpacing: 4,
        leading: IconButton(
          tooltip: 'Close',
          onPressed: () => Navigator.pop(context),
          icon: StrokeIcon(Glyph.close, color: c.text),
        ),
        title: Text(
          widget.slot == null
              ? 'New meal'
              : 'Meal at ${switch (_original) {
                  final PlanView original? => minuteText(context, original.minute),
                  null => time,
                }}',
          style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
        ),
        actions: [
          TextButton(onPressed: _asking ? null : _save, child: const Text('Save')),
          const SizedBox(width: Space.s2),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.all(Space.s4),
        children: [
          OutlinedButton(
            onPressed: _pickTime,
            style: OutlinedButton.styleFrom(minimumSize: const Size(0, 72)),
            child: Text(
              time,
              style: TextStyle(fontSize: Sizes.xl, fontWeight: Fonts.semibold, color: c.text),
            ),
          ),
          const SizedBox(height: Space.s4),
          Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('Portions', style: label),
                    Text('1 to 48', style: muted),
                  ],
                ),
              ),
              IconButton.outlined(
                tooltip: 'One fewer',
                onPressed: _portions > 1 ? () => setState(() => _portions--) : null,
                icon: const StrokeIcon(Glyph.minus),
              ),
              SizedBox(
                width: 56,
                child: Text(
                  '$_portions',
                  textAlign: TextAlign.center,
                  style: TextStyle(fontSize: Sizes.lg, fontWeight: Fonts.semibold, color: c.text),
                ),
              ),
              IconButton.outlined(
                tooltip: 'One more',
                onPressed: _portions < 48 ? () => setState(() => _portions++) : null,
                icon: const StrokeIcon(Glyph.add),
              ),
            ],
          ),
          const SizedBox(height: Space.s4),
          Text('Repeat', style: label),
          const SizedBox(height: Space.s2),
          Segments<List<Weekday>>(
            const [('Daily', _everyDay), ('Weekdays', _weekdays), ('Weekends', _weekend), ('Once', <Weekday>[])],
            selected: same,
            onSelected: (days) => setState(() => _days = days.toSet()),
          ),
          const SizedBox(height: Space.s2),
          // Seven 48dp targets are wider than the narrowest phones, so the last may wrap.
          Wrap(
            alignment: WrapAlignment.spaceBetween,
            children: [
              for (final day in Weekday.values)
                DayToggle(
                  day: day,
                  on: _days.contains(day),
                  onTap: () => setState(() {
                    if (!_days.remove(day)) _days.add(day);
                  }),
                ),
            ],
          ),
          const SizedBox(height: Space.s4),
          Text('Meal call', style: label),
          CallLink(
            onTap: () => unawaited(
              Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => MealCallScreen(feeder: widget.feeder, pick: widget.feeder.pickRecording),
                ),
              ),
            ),
          ),
          const SizedBox(height: Space.s1),
          Text('How many times it plays', style: muted),
          const SizedBox(height: Space.s2),
          Segments<int>(
            const [('Off', 0), ('1', 1), ('2', 2), ('3', 3), ('4', 4), ('5', 5)],
            selected: (plays) => plays == _call,
            onSelected: (plays) => setState(() => _call = plays),
          ),
          // A new meal is on once saved; only a saved one can be turned off.
          if (widget.slot != null) ...[
            const SizedBox(height: Space.s3),
            Row(
              children: [
                // The switch carries the name, so its node is its own size: a
                // tap at the node's center lands on the switch.
                Expanded(
                  child: ExcludeSemantics(child: Text('On', style: label)),
                ),
                Semantics(
                  container: true,
                  label: 'On',
                  child: Switch(value: _enabled, onChanged: (on) => setState(() => _enabled = on)),
                ),
              ],
            ),
          ],
          const SizedBox(height: Space.s2),
          Text(
            'Changes start from the next $time. A meal that already ran '
            "today won't run again.",
            style: muted.copyWith(height: 1.5),
          ),
          if (widget.slot != null) ...[
            const SizedBox(height: Space.s4),
            OutlinedButton(
              style: OutlinedButton.styleFrom(foregroundColor: c.fault),
              onPressed: _asking ? null : _confirmDelete,
              child: const Text('Delete meal'),
            ),
          ],
        ],
      ),
    );
  }
}
