import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/server/pet.dart';
import 'package:wellfed/screens/server/server_words.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/server/household.dart';
import 'package:wellfed/state/server/pet.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/layout.dart';

/// The home's pets and foods, as the owner's server keeps them.
class HouseholdScreen extends StatefulWidget {
  const HouseholdScreen({super.key, required this.flow, required this.newPet, required this.feeders});

  final HouseholdFlow flow;
  final PetFlow? Function(ServerPet pet) newPet;

  /// The server's feeders, for which a pet eats from.
  final List<ListedFeeder> feeders;

  @override
  State<HouseholdScreen> createState() => _HouseholdScreenState();
}

class _HouseholdScreenState extends State<HouseholdScreen> {
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

  Future<void> _openPet(ServerPet pet) async {
    final PetFlow? flow = widget.newPet(pet);
    if (flow == null) return;
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => PetScreen(flow: flow, feeders: widget.feeders),
      ),
    );
    await widget.flow.load();
  }

  Future<void> _addPet() async {
    final PetFields? fields = await askPet(context, null);
    if (fields == null) return;
    await widget.flow.addPet(
      name: fields.name,
      species: fields.species,
      birthDate: fields.birthDate,
      dailyKcal: fields.dailyKcal,
    );
  }

  Future<void> _editFood(ServerFood? food) async {
    final FoodFields? fields = await askFood(context, food);
    if (fields == null) return;
    if (fields.delete && food != null) {
      await widget.flow.deleteFood(food.id);
      return;
    }
    await widget.flow.saveFood(
      id: food?.id,
      brand: fields.brand,
      name: fields.name,
      kcalPerKg: fields.kcalPerKg,
      bagGrams: fields.bagGrams,
    );
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.flow,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final HouseholdState state = widget.flow.state;
      return ServerPage(
        title: 'Pets and foods',
        children: [
          if (state.problem case final problem?) ...[
            Notice(tone: Tone.fault, text: recordProblemWords(problem)),
            const SizedBox(height: Space.s3),
          ],
          const SectionLabel('Pets'),
          if (state.read && state.pets.isEmpty) const Muted('No pets yet.'),
          for (final ServerPet pet in state.pets)
            ListTile(
              contentPadding: EdgeInsets.zero,
              title: Text(pet.name, style: TextStyle(color: c.text)),
              subtitle: Text(speciesWords(pet.species), style: TextStyle(color: c.textMuted)),
              onTap: () => unawaited(_openPet(pet)),
            ),
          TextButton(onPressed: state.busy ? null : () => unawaited(_addPet()), child: const Text('Add a pet')),
          const SizedBox(height: Space.s4),
          const SectionLabel('Foods'),
          if (state.read && state.foods.isEmpty) const Muted('No foods yet.'),
          for (final ServerFood food in state.foods)
            ListTile(
              contentPadding: EdgeInsets.zero,
              title: Text('${food.brand} ${food.name}', style: TextStyle(color: c.text)),
              subtitle: Text(
                '${food.kcalPerKg} kcal/kg · ${food.bagGrams} g bag',
                style: TextStyle(color: c.textMuted),
              ),
              onTap: state.busy ? null : () => unawaited(_editFood(food)),
            ),
          TextButton(onPressed: state.busy ? null : () => unawaited(_editFood(null)), child: const Text('Add a food')),
        ],
      );
    },
  );
}

/// A species, in a word.
String speciesWords(Species species) => switch (species) {
  Species.cat => 'Cat',
  Species.dog => 'Dog',
  Species.other => 'Other',
};

/// A pet as the owner entered it.
typedef PetFields = ({String name, Species species, String? birthDate, int? dailyKcal});

/// A food as the owner entered it, or a choice to remove it.
typedef FoodFields = ({String brand, String name, int kcalPerKg, int bagGrams, bool delete});

/// Ask for a pet's name, species, birth date and daily calories; `pet` is
/// the one changed, `null` for a new one.
Future<PetFields?> askPet(BuildContext context, ServerPet? pet) =>
    showDialog<PetFields>(context: context, builder: (_) => _PetDialog(pet));

/// Ask for a food's brand, name, energy and bag; `food` is the one changed,
/// `null` for a new one.
Future<FoodFields?> askFood(BuildContext context, ServerFood? food) =>
    showDialog<FoodFields>(context: context, builder: (_) => _FoodDialog(food));

class _PetDialog extends StatefulWidget {
  const _PetDialog(this.pet);

  final ServerPet? pet;

  @override
  State<_PetDialog> createState() => _PetDialogState();
}

class _PetDialogState extends State<_PetDialog> {
  late final _name = TextEditingController(text: widget.pet?.name ?? '');
  late final _born = TextEditingController(text: widget.pet?.birthDate ?? '');
  late final _kcal = TextEditingController(text: widget.pet?.dailyKcal?.toString() ?? '');
  late Species _species = widget.pet?.species ?? Species.cat;

  @override
  void dispose() {
    _name.dispose();
    _born.dispose();
    _kcal.dispose();
    super.dispose();
  }

  PetFields? get _fields {
    final String name = _name.text.trim();
    final String born = _born.text.trim();
    final String kcal = _kcal.text.trim();
    final int? daily = int.tryParse(kcal);
    if (name.isEmpty || (kcal.isNotEmpty && daily == null)) return null;
    return (name: name, species: _species, birthDate: born.isEmpty ? null : born, dailyKcal: daily);
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([_name, _born, _kcal]),
    builder: (context, _) => AlertDialog(
      title: Text(widget.pet == null ? 'Add a pet' : 'Edit ${widget.pet?.name}'),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            LabeledField(
              label: 'Name',
              child: TextField(controller: _name),
            ),
            const SizedBox(height: Space.s3),
            SegmentedButton<Species>(
              segments: [
                for (final Species species in Species.values)
                  ButtonSegment(value: species, label: Text(speciesWords(species))),
              ],
              selected: {_species},
              onSelectionChanged: (chosen) => setState(() => _species = chosen.first),
            ),
            const SizedBox(height: Space.s3),
            LabeledField(
              label: 'Born',
              hint: 'Optional, as 2020-04-30',
              child: TextField(controller: _born, keyboardType: TextInputType.datetime),
            ),
            const SizedBox(height: Space.s3),
            LabeledField(
              label: 'Daily calories',
              hint: "Optional: the vet's target, in kcal",
              child: TextField(controller: _kcal, keyboardType: TextInputType.number),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
        TextButton(
          onPressed: switch (_fields) {
            final PetFields fields? => () => Navigator.pop(context, fields),
            null => null,
          },
          child: const Text('Save'),
        ),
      ],
    ),
  );
}

class _FoodDialog extends StatefulWidget {
  const _FoodDialog(this.food);

  final ServerFood? food;

  @override
  State<_FoodDialog> createState() => _FoodDialogState();
}

class _FoodDialogState extends State<_FoodDialog> {
  late final _brand = TextEditingController(text: widget.food?.brand ?? '');
  late final _name = TextEditingController(text: widget.food?.name ?? '');
  late final _kcal = TextEditingController(text: widget.food?.kcalPerKg.toString() ?? '');
  late final _bag = TextEditingController(text: widget.food?.bagGrams.toString() ?? '');

  @override
  void dispose() {
    _brand.dispose();
    _name.dispose();
    _kcal.dispose();
    _bag.dispose();
    super.dispose();
  }

  FoodFields? get _fields {
    final int? kcal = int.tryParse(_kcal.text.trim());
    final int? bag = int.tryParse(_bag.text.trim());
    final String brand = _brand.text.trim();
    final String name = _name.text.trim();
    if (brand.isEmpty || name.isEmpty || kcal == null || bag == null) return null;
    return (brand: brand, name: name, kcalPerKg: kcal, bagGrams: bag, delete: false);
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([_brand, _name, _kcal, _bag]),
    builder: (context, _) {
      final ServerFood? food = widget.food;
      return AlertDialog(
        title: Text(food == null ? 'Add a food' : 'Edit ${food.name}'),
        content: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              LabeledField(
                label: 'Brand',
                child: TextField(controller: _brand),
              ),
              const SizedBox(height: Space.s3),
              LabeledField(
                label: 'Name',
                child: TextField(controller: _name),
              ),
              const SizedBox(height: Space.s3),
              LabeledField(
                label: 'Energy',
                hint: 'kcal per kg, from the bag',
                child: TextField(controller: _kcal, keyboardType: TextInputType.number),
              ),
              const SizedBox(height: Space.s3),
              LabeledField(
                label: 'Bag',
                hint: 'grams in a full bag',
                child: TextField(controller: _bag, keyboardType: TextInputType.number),
              ),
            ],
          ),
        ),
        actions: [
          if (food != null)
            TextButton(
              onPressed: () => Navigator.pop(context, (
                brand: food.brand,
                name: food.name,
                kcalPerKg: food.kcalPerKg,
                bagGrams: food.bagGrams,
                delete: true,
              )),
              child: const Text('Remove'),
            ),
          TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
          TextButton(
            onPressed: switch (_fields) {
              final FoodFields fields? => () => Navigator.pop(context, fields),
              null => null,
            },
            child: const Text('Save'),
          ),
        ],
      );
    },
  );
}
