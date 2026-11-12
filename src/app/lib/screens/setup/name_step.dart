import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:wellfed/src/limits.g.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

/// Name things: what this phone calls the feeder (only this phone keeps
/// it) and how the feeder lists this phone among its clients, which it
/// keeps in at most [Limits.labelBytesMost] bytes.
class NameStep extends StatefulWidget {
  const NameStep({
    super.key,
    required this.initial,
    required this.onDone,
    required this.action,
    this.feeder,
    this.stepLabel,
    this.progress,
    this.onBack,
    required this.suggest,
  });

  /// The phone's name so far; empty until the owner names it.
  final String initial;

  /// The feeder's name so far, or `null` to not ask for one.
  final String? feeder;
  final String action;
  final String? stepLabel;
  final double? progress;
  final VoidCallback? onBack;

  /// A name for a phone that has none yet.
  final Future<String?> Function() suggest;

  /// The phone's name, then the feeder's when it was asked; why it failed, which
  /// keeps the owner on this step, or null.
  final Future<String?> Function(String label, String? feeder) onDone;

  @override
  State<NameStep> createState() => _NameStepState();
}

class _NameStepState extends State<NameStep> {
  final _name = TextEditingController();
  late final _feeder = TextEditingController(text: widget.feeder);

  /// The owner's name could not be saved; stays on this step to say so.
  String? _failed;

  @override
  void initState() {
    super.initState();
    _name.text = widget.initial;
    if (widget.initial.isEmpty) unawaited(_suggest());
  }

  /// The phone's model, cut to fit.
  Future<void> _suggest() async {
    final String? suggested = await widget.suggest();
    if (suggested == null || !mounted || _name.text.isNotEmpty) return;
    String model = suggested;
    while (utf8.encode(model).length > Limits.labelBytesMost) {
      model = model.substring(0, model.length - 1);
    }
    setState(() => _name.text = model);
  }

  @override
  void dispose() {
    _name.dispose();
    _feeder.dispose();
    super.dispose();
  }

  int get _bytes => utf8.encode(_name.text.trim()).length;

  /// The names are being saved: a second tap would start a second run.
  bool _busy = false;

  Future<void> _done() async {
    if (_busy) return;
    setState(() => _busy = true);
    final asksFeeder = widget.feeder != null;
    final String? failed = await widget.onDone(_name.text.trim(), asksFeeder ? _feeder.text.trim() : null);
    if (!mounted) return;
    setState(() {
      _busy = false;
      if (failed != null) _failed = failed;
    });
  }

  bool get _valid =>
      _bytes >= 1 && _bytes <= Limits.labelBytesMost && (widget.feeder == null || _feeder.text.trim().isNotEmpty);

  @override
  Widget build(BuildContext context) {
    final asksFeeder = widget.feeder != null;
    return StepScaffold(
      stepLabel: widget.stepLabel,
      progress: widget.progress,
      onLeading: widget.onBack,
      title: asksFeeder ? 'Name things' : 'Name this $deviceName',
      content: [
        if (asksFeeder) ...[
          LabeledField(
            label: 'This feeder',
            hint: 'Only this $deviceName uses it.',
            child: TextField(
              controller: _feeder,
              textCapitalization: TextCapitalization.words,
              textInputAction: TextInputAction.next,
              onChanged: (_) => setState(() {}),
            ),
          ),
          const SizedBox(height: 20),
        ],
        LabeledField(
          label: 'This $deviceName, as the feeder shows it',
          hint: "Others see it in the feeder's People list and its history.",
          counter: '$_bytes/${Limits.labelBytesMost}',
          child: TextField(
            controller: _name,
            autofocus: widget.initial.isEmpty && !asksFeeder,
            textInputAction: TextInputAction.done,
            onChanged: (_) => setState(() {}),
          ),
        ),
        const SizedBox(height: 20),
        Notice(
          tone: Tone.neutral,
          glyph: Glyph.key,
          radius: Radii.card,
          text:
              "You'll be this feeder's **owner**. Its key stays on this "
              "$deviceName; there's no account to sign in to.",
        ),
        if (_failed case final failed?) ...[
          const SizedBox(height: Space.s3),
          Notice(tone: Tone.fault, lead: "This $deviceName couldn't save that.", text: failed),
        ],
      ],
      actions: [
        FilledButton(onPressed: _valid && !_busy ? () => unawaited(_done()) : null, child: Text(widget.action)),
      ],
    );
  }
}
