import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/server/server_words.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/server/feeder_food.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/ask_text.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/when.dart';

/// One feeder's food, as the owner's server keeps it: what it holds, what
/// went since the last refill, and the refills. The portion calibration is
/// the only weight: until it is set this counts portions, with no grams,
/// calories or food left.
class FeederFoodScreen extends StatefulWidget {
  const FeederFoodScreen({super.key, required this.flow, required this.name});

  final FeederFoodFlow flow;
  final String name;

  @override
  State<FeederFoodScreen> createState() => _FeederFoodScreenState();
}

class _FeederFoodScreenState extends State<FeederFoodScreen> {
  @override
  void initState() {
    super.initState();
    unawaited(widget.flow.load());
  }

  @override
  void dispose() {
    widget.flow.dispose();
    super.dispose();
  }

  Future<void> _chooseFood() async {
    final List<ServerFood> foods = widget.flow.state.foods;
    final String? id = await showDialog<String>(
      context: context,
      builder: (context) => SimpleDialog(
        title: const Text('What it holds'),
        children: [
          if (foods.isEmpty)
            const Padding(padding: EdgeInsets.all(Space.s4), child: Text('Add a food under Pets and foods first.')),
          for (final ServerFood food in foods)
            SimpleDialogOption(
              onPressed: () => Navigator.pop(context, food.id),
              child: Text('${food.brand} ${food.name}'),
            ),
        ],
      ),
    );
    if (id != null) await widget.flow.setFood(id);
  }

  Future<void> _refill({required bool fullBag}) async {
    if (fullBag) {
      await widget.flow.addRefill(const NewRefill.fullBag());
      return;
    }
    final String? text = await askText(
      context,
      title: 'Record a refill',
      label: 'Grams poured in',
      keyboard: TextInputType.number,
      action: 'Record',
    );
    final int? grams = int.tryParse((text ?? '').trim());
    if (grams == null || grams <= 0) return;
    await widget.flow.addRefill(NewRefill.grams(grams: grams));
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.flow,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final FeederFoodState state = widget.flow.state;
      final ServerFeederFood? food = state.food;
      final bool calibrated = state.calibrated;
      final ServerSinceRefill? since = food?.sinceRefill;
      final ServerFoodLeft? left = calibrated ? since?.left : null;
      final FeedRate? rate = food?.rate;
      return ServerPage(
        title: '${widget.name}: food',
        children: [
          if (state.error case final error?) ...[
            Notice(tone: Tone.fault, text: serverErrorWords(error)),
            const SizedBox(height: Space.s3),
          ],
          if (food == null && state.error == null) const Muted('Reading…'),
          if (food != null) ...[
            const SectionLabel('Holds'),
            Text(switch (food.current) {
              final ServerFoodPeriod period? => '${period.food.brand} ${period.food.name}',
              null => 'No food set',
            }, style: TextStyle(fontSize: Sizes.md, color: c.text)),
            Align(
              alignment: AlignmentDirectional.centerStart,
              child: TextButton(
                onPressed: state.busy ? null : () => unawaited(_chooseFood()),
                child: const Text('Change the food'),
              ),
            ),
            if (!calibrated) ...[
              const Notice(
                tone: Tone.neutral,
                text: "Set this feeder's portion size to see grams, calories and food left. Until then it counts portions.",
              ),
              const SizedBox(height: Space.s3),
            ],
            if (food.refilledUnrecordedMs case final int at?) ...[
              Notice(
                tone: Tone.warning,
                text: 'The feeder saw food come back ${atMomentTextMs(context, at)} with no refill recorded.',
              ),
              const SizedBox(height: Space.s3),
            ],
            const SectionLabel('Since the last refill'),
            if (since == null) const Muted('No refill recorded yet.'),
            if (since != null)
              Muted(
                [
                  portionsText(since.portions),
                  if (since.uncertainPortions > 0) '${since.uncertainPortions} uncertain',
                  if (left != null) '${left.gramsLeft} g left',
                  if (left?.emptyAtMs case final int empty?) 'empty ${atMomentTextMs(context, empty)}',
                ].join(' · '),
              ),
            if (rate != null)
              Muted(
                calibrated && rate.gramsPerDay != null
                    ? '${rate.gramsPerDay} g a day'
                    : '${portionsText(rate.portions)} in ${rate.hours} hours',
              ),
            const SizedBox(height: Space.s4),
            const SectionLabel('Refills'),
            Row(
              children: [
                TextButton(
                  onPressed: state.busy ? null : () => unawaited(_refill(fullBag: true)),
                  child: const Text('Full bag'),
                ),
                TextButton(
                  onPressed: state.busy ? null : () => unawaited(_refill(fullBag: false)),
                  child: const Text('Some grams'),
                ),
              ],
            ),
            for (final ServerRefill refill in state.refills)
              ListTile(
                contentPadding: EdgeInsets.zero,
                title: Text(refill.fullBag ? 'Full bag' : '${refill.grams} g', style: TextStyle(color: c.text)),
                subtitle: Text(momentTextMs(context, refill.atMs), style: TextStyle(color: c.textMuted)),
                trailing: IconButton(
                  tooltip: 'Remove this refill',
                  icon: Icon(Icons.delete_outline, color: c.textMuted),
                  onPressed: state.busy ? null : () => unawaited(widget.flow.deleteRefill(refill.id)),
                ),
              ),
          ],
        ],
      );
    },
  );
}
