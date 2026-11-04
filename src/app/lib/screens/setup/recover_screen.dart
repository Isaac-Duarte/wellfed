import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/setup/name_step.dart';
import 'package:wellfed/screens/setup/pairing_view.dart';
import 'package:wellfed/state/pairing/pairing.dart';
import 'package:wellfed/state/setup/setup.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/code_tiles.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

/// LAN recovery: a feeder already on this network takes this phone as an
/// owner while its window is open (Lock, then Dispense, held).
class RecoverScreen extends StatefulWidget {
  const RecoverScreen({super.key, required this.newFlow});

  /// The controller of this screen's one recovery.
  final RecoverFlow Function() newFlow;

  @override
  State<RecoverScreen> createState() => _RecoverScreenState();
}

class _RecoverScreenState extends State<RecoverScreen> {
  late final RecoverFlow _flow = widget.newFlow();
  (String, String)? _picked;

  @override
  void dispose() {
    _flow.dispose();
    super.dispose();
  }

  /// Pair with the feeder at `address`, saved under `name`.
  void _pair(String address, String name) {
    _picked = (address, name);
    unawaited(_flow.pair(address, name));
  }

  void _again() {
    if (_picked case (final String address, final String name)) _pair(address, name);
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _flow,
    builder: (context, _) => switch (_flow.state) {
      RecoverName() => NameStep(
        initial: _flow.label,
        action: 'Continue',
        suggest: _flow.phoneModel,
        onDone: (label, _) => _flow.named(label),
      ),
      RecoverPick(:final List<FoundFeeder> found) => _Pick(found: found, onPicked: _pair),
      RecoverPairing(:final PairingFlow pairing) => PairingView(
        flow: pairing,
        ble: false,
        onRetry: _again,
        onNewCode: _again,
      ),
    },
  );
}

/// How to open the window, then the feeders answering on this network, or
/// an address typed in.
class _Pick extends StatefulWidget {
  const _Pick({required this.found, required this.onPicked});

  /// The feeders answering on this network.
  final List<FoundFeeder> found;

  final void Function(String address, String name) onPicked;

  @override
  State<_Pick> createState() => _PickState();
}

class _PickState extends State<_Pick> {
  final _typed = TextEditingController();

  @override
  void dispose() {
    _typed.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return StepScaffold(
      title: 'Recover a feeder',
      body:
          'Lost every phone that could open it? If the feeder is still on '
          'your Wi-Fi, you can add this phone as an owner without erasing '
          'anything.',
      content: [
        const PanelPhoto(held: [1, 2], caption: 'Lock, then Dispense'),
        const SizedBox(height: 18),
        const NumberedSteps([
          'Unlock the panel if the lock light is on.',
          'Hold **Lock**, then also **Dispense**, for 5 seconds. The lock light blinks fast.',
          'Within 2 minutes, pick the feeder below and press the code it shows, just like setup.',
        ]),
        const SizedBox(height: Space.s5),
        if (widget.found.isEmpty) const Searching('Looking for feeders on this network…'),
        for (final feeder in widget.found) ...[
          Panel(
            onTap: () => widget.onPicked(feeder.address, feeder.name),
            padding: const EdgeInsets.all(Space.s4),
            child: Row(
              children: [
                const FeederImage(size: 48),
                const SizedBox(width: Space.s3),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        feeder.name,
                        style: TextStyle(fontSize: Sizes.md, fontWeight: Fonts.semibold, color: c.text),
                      ),
                      const SizedBox(height: 2),
                      Text(
                        feeder.address,
                        style: TextStyle(fontFamily: Fonts.mono, fontSize: Sizes.sm, color: c.textMuted),
                      ),
                    ],
                  ),
                ),
                StrokeIcon(Glyph.chevron, size: 20, color: c.textMuted),
              ],
            ),
          ),
          const SizedBox(height: Space.s2),
        ],
        const SizedBox(height: Space.s5),
        LabeledField(
          label: 'Or its address',
          child: TextField(
            controller: _typed,
            keyboardType: TextInputType.url,
            decoration: const InputDecoration(hintText: '192.168.1.20'),
            onChanged: (_) => setState(() {}),
          ),
        ),
      ],
      actions: [
        OutlinedButton(
          onPressed: _typed.text.trim().isEmpty
              ? null
              : () => widget.onPicked(_typed.text.trim(), PairingFlow.defaultFeederName),
          child: const Text('Use this address'),
        ),
      ],
    );
  }
}
