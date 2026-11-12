import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/server/household.dart';
import 'package:wellfed/screens/server/server_words.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/server/pet.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/ask_text.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/when.dart';

/// One pet: what it ate day by day, its weights and the feeders it eats
/// from. Grams and calories show only for feeders whose portion size is set;
/// until then the days count portions.
class PetScreen extends StatefulWidget {
  const PetScreen({super.key, required this.flow, required this.feeders});

  final PetFlow flow;

  /// The server's feeders, to choose which the pet eats from.
  final List<ListedFeeder> feeders;

  @override
  State<PetScreen> createState() => _PetScreenState();
}

class _PetScreenState extends State<PetScreen> {
  @override
  void initState() {
    super.initState();
    widget.flow.addListener(_changed);
    unawaited(widget.flow.load());
  }

  @override
  void dispose() {
    widget.flow
      ..removeListener(_changed)
      ..dispose();
    super.dispose();
  }

  bool _left = false;

  /// The server removed the pet: nothing is left to show. The screen stays
  /// mounted while it slides away, and later changes must not pop what lies
  /// beneath it.
  void _changed() {
    if (_left || !widget.flow.state.removed || !mounted) return;
    _left = true;
    Navigator.pop(context);
  }

  Future<void> _edit() async {
    final ServerPet pet = widget.flow.state.pet;
    final PetFields? fields = await askPet(context, pet);
    if (fields == null) return;
    await widget.flow.save(
      name: fields.name,
      species: fields.species,
      birthDate: fields.birthDate,
      dailyKcal: fields.dailyKcal,
    );
  }

  Future<void> _delete() async {
    final ServerPet pet = widget.flow.state.pet;
    final bool sure =
        await showDialog<bool>(
          context: context,
          builder: (context) => AlertDialog(
            title: Text('Remove ${pet.name}?'),
            content: const Text('Its weights go with it. The feeders and their history stay.'),
            actions: [
              TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
              TextButton(onPressed: () => Navigator.pop(context, true), child: const Text('Remove')),
            ],
          ),
        ) ??
        false;
    if (!sure) return;
    await widget.flow.remove();
  }

  Future<void> _weigh() async {
    final String? text = await askText(
      context,
      title: 'Log a weight',
      label: 'Weight in kg',
      keyboard: const TextInputType.numberWithOptions(decimal: true),
      action: 'Log',
    );
    final double? kg = double.tryParse((text ?? '').trim().replaceAll(',', '.'));
    if (kg == null || kg <= 0) return;
    await widget.flow.addWeight((kg * 1000).round());
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.flow,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final PetState state = widget.flow.state;
      final ServerPet pet = state.pet;
      final ServerPetIntake? intake = state.intake;
      return ServerPage(
        title: pet.name,
        children: [
          if (state.problem case final problem?) ...[
            Notice(tone: Tone.fault, text: recordProblemWords(problem)),
            const SizedBox(height: Space.s3),
          ],
          Muted(
            [
              speciesWords(pet.species),
              if (pet.birthDate case final String born?) 'born $born',
              if (pet.dailyKcal case final int kcal?) '$kcal kcal a day',
            ].join(' · '),
          ),
          Row(
            children: [
              TextButton(onPressed: () => unawaited(_edit()), child: const Text('Edit')),
              TextButton(onPressed: () => unawaited(_delete()), child: const Text('Remove')),
            ],
          ),
          const SizedBox(height: Space.s3),
          const SectionLabel('What it ate'),
          if (intake == null) const Muted('Reading…'),
          if (intake != null && intake.days.isEmpty) const Muted('Nothing yet.'),
          if (intake != null)
            for (final ServerIntakeDay day in intake.days.reversed) _IntakeRow(day: day, feeders: widget.feeders),
          const SizedBox(height: Space.s4),
          const SectionLabel('Eats from'),
          for (final ListedFeeder feeder in widget.feeders)
            CheckboxListTile(
              contentPadding: EdgeInsets.zero,
              title: Text(feeder.name, style: TextStyle(color: c.text)),
              value: pet.feeders.contains(feeder.id),
              onChanged: state.busy ? null : (eats) => unawaited(widget.flow.eatsFrom(feeder.id, eats: eats ?? false)),
            ),
          const SizedBox(height: Space.s4),
          const SectionLabel('Weight'),
          if (state.weights.isEmpty) const Muted('No weights logged.'),
          for (final ServerWeight weight in state.weights)
            ListTile(
              contentPadding: EdgeInsets.zero,
              title: Text('${(weight.grams / 1000).toStringAsFixed(2)} kg', style: TextStyle(color: c.text)),
              subtitle: Text(momentTextMs(context, weight.atMs), style: TextStyle(color: c.textMuted)),
              trailing: IconButton(
                tooltip: 'Remove this weight',
                icon: Icon(Icons.delete_outline, color: c.textMuted),
                onPressed: state.busy ? null : () => unawaited(widget.flow.deleteWeight(weight.id)),
              ),
            ),
          TextButton(onPressed: state.busy ? null : () => unawaited(_weigh()), child: const Text('Log a weight')),
        ],
      );
    },
  );
}

/// One day of a pet's intake: grams and calories from calibrated feeders,
/// and portions from the rest, which weigh nothing until calibrated.
class _IntakeRow extends StatelessWidget {
  const _IntakeRow({required this.day, required this.feeders});

  final ServerIntakeDay day;
  final List<ListedFeeder> feeders;

  String _name(String id) => feeders.where((feeder) => feeder.id == id).firstOrNull?.name ?? 'A feeder';

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final WeighedIntake? weighed = day.weighed;
    final List<String> lines = [
      if (weighed != null)
        [
          '${weighed.grams} g',
          '${weighed.kcal} kcal',
          if (day.targetPercent case final int percent?) '$percent% of target',
          if (day.estimate) 'estimate',
        ].join(' · '),
      for (final ServerFeederPortions portions in day.unweighed)
        [
          '${portionsText(portions.portions)} from ${_name(portions.feeder)}',
          if (portions.uncertainPortions > 0) '${portions.uncertainPortions} uncertain',
        ].join(', '),
    ];
    return ListTile(
      contentPadding: EdgeInsets.zero,
      title: Text(day.date, style: TextStyle(color: c.text)),
      subtitle: Text(lines.isEmpty ? 'Nothing' : lines.join('\n'), style: TextStyle(color: c.textMuted)),
    );
  }
}
