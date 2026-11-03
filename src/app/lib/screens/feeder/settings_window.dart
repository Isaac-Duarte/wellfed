import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/pending.dart';
import 'package:wellfed/widgets/settings_list.dart';
import 'package:wellfed/widgets/when.dart';

/// What a window comes to, in words.
String windowText(BuildContext context, Window window) => switch (window) {
  Window_Always() => 'Any time',
  Window_Between(:final start, :final end) => '${minuteText(context, start)} to ${minuteText(context, end)}',
};

class SettingsWindowRow extends StatelessWidget {
  const SettingsWindowRow({
    super.key,
    required this.feeder,
    required this.settings,
    required this.online,
    required this.name,
    required this.on,
    required this.window,
    required this.question,
    required this.toggle,
    required this.windowed,
    this.first = false,
  });

  final FeederController feeder;
  final SettingsView settings;
  final bool online;
  final String name;
  final bool on;
  final Window window;
  final String question;
  final SettingsChanges Function(bool) toggle;
  final SettingsChanges Function(Window) windowed;
  final bool first;

  @override
  Widget build(BuildContext context) => Pending<Window>(
    card: window,
    generation: settings.generation,
    builder: (context, shown, windowWrite) => SettingsTile(
      first: first,
      onTap: online
          ? () async {
              final Window? chosen = await showDialog<Window>(
                context: context,
                builder: (_) => _WindowDialog(question: question, window: shown),
              );
              if (chosen != null && context.mounted) {
                await windowWrite.ask(chosen, () => put(feeder, settings, windowed(chosen)));
              }
            }
          : null,
      child: Row(
        children: [
          Expanded(child: SettingsWords(name, windowText(context, shown))),
          Pending<bool>(
            card: on,
            generation: settings.generation,
            builder: (context, shownOn, onWrite) => Semantics(
              container: true,
              label: name,
              child: Switch(
                value: shownOn,
                onChanged: online
                    ? (next) => unawaited(onWrite.ask(next, () => put(feeder, settings, toggle(next))))
                    : null,
              ),
            ),
          ),
        ],
      ),
    ),
  );
}

/// When the lights or sounds may be on (10.2).
class _WindowDialog extends StatefulWidget {
  const _WindowDialog({required this.question, required this.window});

  final String question;
  final Window window;

  @override
  State<_WindowDialog> createState() => _WindowDialogState();
}

class _WindowDialogState extends State<_WindowDialog> {
  late bool _between = widget.window is Window_Between;
  late int _start = switch (widget.window) {
    Window_Between(:final int start) => start,
    _ => 7 * 60,
  };
  late int _end = switch (widget.window) {
    Window_Between(:final int end) => end,
    _ => 22 * 60,
  };

  Future<void> _pick(int minute, void Function(int) chosen) async {
    final TimeOfDay? picked = await showTimePicker(
      context: context,
      initialTime: TimeOfDay(hour: minute ~/ 60, minute: minute % 60),
    );
    if (picked != null) {
      setState(() => chosen(picked.hour * 60 + picked.minute));
    }
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    scrollable: true,
    title: Text(widget.question),
    content: Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        RadioGroup<bool>(
          groupValue: _between,
          onChanged: (between) => setState(() => _between = between ?? false),
          child: const Column(
            children: [
              RadioListTile<bool>(value: false, contentPadding: EdgeInsets.zero, title: Text('Any time')),
              RadioListTile<bool>(value: true, contentPadding: EdgeInsets.zero, title: Text('Only between')),
            ],
          ),
        ),
        if (_between)
          Row(
            children: [
              Expanded(
                child: OutlinedButton(
                  onPressed: () => unawaited(_pick(_start, (minute) => _start = minute)),
                  child: Text(minuteText(context, _start)),
                ),
              ),
              const Padding(
                padding: EdgeInsets.symmetric(horizontal: Space.s2),
                child: Text('and'),
              ),
              Expanded(
                child: OutlinedButton(
                  onPressed: () => unawaited(_pick(_end, (minute) => _end = minute)),
                  child: Text(minuteText(context, _end)),
                ),
              ),
            ],
          ),
        const SizedBox(height: Space.s3),
        Text(
          'A window can cross midnight. Until the feeder knows the time, it '
          'treats every window as open.',
          style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: context.colors.textMuted),
        ),
      ],
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(
        onPressed: () =>
            Navigator.pop(context, _between ? Window.between(start: _start, end: _end) : const Window.always()),
        child: const Text('Save'),
      ),
    ],
  );
}
