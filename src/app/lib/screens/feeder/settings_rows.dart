import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/pending.dart';
import 'package:wellfed/widgets/replies.dart';
import 'package:wellfed/widgets/settings_list.dart';

class SettingsVolumeRow extends StatelessWidget {
  const SettingsVolumeRow({
    super.key,
    required this.feeder,
    required this.settings,
    required this.online,
    required this.canChange,
  });

  final FeederController feeder;
  final SettingsView settings;

  /// The test sound plays while online: it changes no setting.
  final bool online;

  /// Online, and the settings writable: a change can go.
  final bool canChange;

  Future<void> _play(BuildContext context) async {
    final FeederReply reply = await feeder.ask(const FeederRequest.play(tune: Tune.test));
    if (context.mounted) showProblem(context, reply);
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    // Where the finger holds it, then the volume asked for until the card
    // shows it: the slider never springs back to the old value on the way.
    return Pending<double>(
      card: settings.volume.toDouble(),
      generation: settings.generation,
      builder: (context, volume, write) => SettingsTile(
        first: false,
        child: Row(
          children: [
            Text('Volume', style: TextStyle(fontSize: 15, color: c.text)),
            Expanded(
              child: Slider(
                value: volume,
                max: 100,
                onChanged: canChange ? write.hold : null,
                onChangeEnd: (end) {
                  final int asked = end.round();
                  unawaited(write.ask(asked.toDouble(), () => put(feeder, settings, SettingsChanges(volume: asked))));
                },
              ),
            ),
            SizedBox(
              width: 28,
              child: Text(
                '${volume.round()}',
                textAlign: TextAlign.end,
                style: TextStyle(fontSize: Sizes.base, color: c.text),
              ),
            ),
            IconButton(
              tooltip: 'Play a test sound',
              onPressed: online ? () => unawaited(_play(context)) : null,
              icon: Icon(Icons.volume_up_outlined, color: c.textMuted),
            ),
          ],
        ),
      ),
    );
  }
}

class SettingsReminderRow extends StatelessWidget {
  const SettingsReminderRow({super.key, required this.feeder, required this.settings, required this.canChange});

  final FeederController feeder;
  final SettingsView settings;

  /// Online, and the settings writable: a change can go.
  final bool canChange;

  @override
  Widget build(BuildContext context) => Pending<MealReminder>(
    card: settings.mealReminder,
    generation: settings.generation,
    builder: (context, reminder, write) => SettingsTile(
      first: true,
      onTap: canChange
          ? () async {
              final MealReminder? chosen = await showDialog<MealReminder>(
                context: context,
                builder: (context) => SimpleDialog(
                  title: const Text('Before each meal'),
                  children: [
                    for (final (name, value) in const [
                      ('Off', MealReminder.off()),
                      ('5 minutes ahead', MealReminder.before(minutes: 5)),
                      ('15 minutes ahead', MealReminder.before(minutes: 15)),
                      ('30 minutes ahead', MealReminder.before(minutes: 30)),
                      ('60 minutes ahead', MealReminder.before(minutes: 60)),
                    ])
                      SimpleDialogOption(onPressed: () => Navigator.pop(context, value), child: Text(name)),
                  ],
                ),
              );
              if (chosen != null && context.mounted) {
                await write.ask(chosen, () => put(feeder, settings, SettingsChanges(mealReminder: chosen)));
              }
            }
          : null,
      child: Row(
        children: [
          Expanded(
            child: SettingsWords('Before each meal', switch (reminder) {
              MealReminder_Off() => 'Off',
              MealReminder_Before(:final minutes) => '$minutes minutes ahead',
              MealReminder_Unknown() => 'Unknown to this app',
            }),
          ),
          StrokeIcon(Glyph.chevron, size: 20, color: context.colors.textMuted),
        ],
      ),
    ),
  );
}

class SettingsDesiccantRow extends StatefulWidget {
  const SettingsDesiccantRow({super.key, required this.feeder, required this.settings, required this.canChange});

  final FeederController feeder;
  final SettingsView settings;

  /// Online, and the settings writable: a change can go.
  final bool canChange;

  @override
  State<SettingsDesiccantRow> createState() => _DesiccantRowState();
}

class _DesiccantRowState extends State<SettingsDesiccantRow> {
  bool _asking = false;

  Future<void> _changed() async {
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(const FeederRequest.resetDesiccant());
    if (!mounted) return;
    setState(() => _asking = false);
    showProblem(context, reply);
  }

  Future<void> _period(PendingWrite<int> write) async {
    final int? days = await showDialog<int>(
      context: context,
      builder: (context) => SimpleDialog(
        title: const Text('Change the desiccant every'),
        children: [
          for (final days in const [7, 14, 30, 60, 90])
            SimpleDialogOption(onPressed: () => Navigator.pop(context, days), child: Text('$days days')),
        ],
      ),
    );
    if (days != null && mounted) {
      await write.ask(days, () => put(widget.feeder, widget.settings, SettingsChanges(desiccantDays: days)));
    }
  }

  @override
  Widget build(BuildContext context) {
    final Desiccant? desiccant = widget.feeder.state.card?.desiccant;
    final String? note = switch (desiccant) {
      Desiccant_Remaining(days: 0) => 'Change today',
      Desiccant_Remaining(days: 1) => 'Next change in 1 day',
      Desiccant_Remaining(:final days) when days > 1 => 'Next change in $days days',
      Desiccant_Remaining(:final days) => '${-days} days overdue',
      Desiccant_Unset() => 'Not started',
      Desiccant_Unknown() => 'Unknown until the clock is set',
      null => null,
    };
    return Pending<int>(
      card: widget.settings.desiccantDays,
      generation: widget.settings.generation,
      builder: (context, period, write) => SettingsTile(
        first: false,
        onTap: widget.canChange ? () => unawaited(_period(write)) : null,
        // The button drops under the words when large text leaves no room beside them.
        child: SizedBox(
          width: double.infinity,
          child: Wrap(
            alignment: WrapAlignment.spaceBetween,
            crossAxisAlignment: WrapCrossAlignment.center,
            spacing: Space.s3,
            runSpacing: Space.s2,
            children: [
              SettingsWords('Desiccant every $period days', note),
              TextButton(
                style: TextButton.styleFrom(
                  minimumSize: const Size(0, 48),
                  padding: const EdgeInsets.symmetric(horizontal: Space.s3),
                ),
                onPressed: widget.canChange && !_asking ? _changed : null,
                child: const Text('Mark changed'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
