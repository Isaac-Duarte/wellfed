import 'dart:async';

import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/join/join.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/ask_text.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/replies.dart';

/// Join a feeder with someone's invite (4.1 to 4.3). An invite works once, so
/// a join whose answer never came is not sent again: the screen says so and
/// asks for a new one.
class JoinScreen extends StatefulWidget {
  const JoinScreen({super.key, required this.newFlow, this.scanner});

  /// The controller of this screen's one join.
  final JoinFlow Function() newFlow;

  /// What reads a code in place of the camera, handing on each text it sees.
  final Widget Function(void Function(String))? scanner;

  @override
  State<JoinScreen> createState() => _JoinScreenState();
}

class _JoinScreenState extends State<JoinScreen> {
  late final JoinFlow _flow = widget.newFlow();
  late final _feederName = TextEditingController(text: 'Feeder');
  late final _phoneName = TextEditingController(text: _flow.label);

  @override
  void dispose() {
    _flow.dispose();
    _feederName.dispose();
    _phoneName.dispose();
    super.dispose();
  }

  Future<void> _paste() async {
    final String? text = await askText(context, title: 'Paste an invite link', label: 'Invite', action: 'Use it');
    if (text != null && mounted) _flow.seen(text);
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _flow,
    builder: (context, _) => switch (_flow.state) {
      final JoinScanning state => Builder(
        builder: (context) {
          final ThemeTokens c = context.colors;
          return StepScaffold(
            title: 'Join with an invite',
            body:
                'Ask an owner to open People › Invite someone on their phone, then '
                'point this camera at the code.',
            content: [
              ClipRRect(
                borderRadius: BorderRadius.circular(16),
                child: SizedBox(height: 300, child: widget.scanner?.call(_flow.seen) ?? _Camera(_flow.seen)),
              ),
              if (state.badText case final String bad?)
                Padding(
                  padding: const EdgeInsets.only(top: Space.s3),
                  child: Text(bad, style: TextStyle(color: c.fault)),
                ),
            ],
            actions: [OutlinedButton(onPressed: () => unawaited(_paste()), child: const Text('Paste an invite link'))],
          );
        },
      ),
      final JoinReview state => Builder(
        builder: (context) {
          final bool joining = state.joining;
          final FeederRefusal? refusal = state.refusal;
          final String? problem = refusal == null ? state.problem : refusalWords(refusal);
          final where = state.finding
              ? 'Looking…'
              : state.address == null
              ? 'Not found'
              : 'Found';
          return StepScaffold(
            title: "You're invited to a feeder",
            content: [
              FactList([('On your Wi-Fi', Text(where))]),
              if (state.address == null && !state.finding) ...[
                const SizedBox(height: Space.s3),
                const Notice(
                  tone: Tone.neutral,
                  glyph: Glyph.wifi,
                  text: 'Join the same Wi-Fi as the feeder, then look again.',
                ),
                const SizedBox(height: Space.s2),
                OutlinedButton(onPressed: () => unawaited(_flow.find()), child: const Text('Look again')),
              ],
              const SizedBox(height: Space.s4),
              LabeledField(
                label: 'This feeder',
                hint: 'Only this phone uses it.',
                child: TextField(controller: _feederName, onChanged: (_) => setState(() {})),
              ),
              const SizedBox(height: Space.s4),
              LabeledField(
                label: 'This phone',
                hint: 'The feeder lists you by this.',
                child: TextField(controller: _phoneName, onChanged: (_) => setState(() {})),
              ),
              if (problem != null) ...[
                const SizedBox(height: Space.s4),
                Notice(tone: Tone.fault, glyph: Glyph.warning, text: problem),
              ],
            ],
            actions: [
              FilledButton(
                onPressed:
                    joining ||
                        state.spent ||
                        state.address == null ||
                        _feederName.text.trim().isEmpty ||
                        _phoneName.text.trim().isEmpty
                    ? null
                    : () => unawaited(_flow.join(label: _phoneName.text.trim(), feederName: _feederName.text.trim())),
                child: Text(joining ? 'Joining…' : 'Join ${_feederName.text.trim()}'),
              ),
            ],
          );
        },
      ),
      final JoinJoined state => Builder(
        builder: (context) {
          final String name = _feederName.text.trim();
          final owner = state.role == ClientRole.owner;
          return StepScaffold(
            leading: Leading.none,
            title: 'You can use $name',
            body: owner
                ? 'You can do everything here, including Wi-Fi, updates and '
                      'removing people.'
                : 'You can feed, edit meals and settings, and lock the buttons. '
                      'Wi-Fi, updates and removing people stay with its owners.',
            actions: [
              FilledButton(
                onPressed: () => Navigator.of(context).popUntil((route) => route.isFirst),
                child: const Text('Done'),
              ),
            ],
          );
        },
      ),
    },
  );
}

class _Camera extends StatelessWidget {
  const _Camera(this.seen);

  final void Function(String) seen;

  @override
  Widget build(BuildContext context) => MobileScanner(
    onDetect: (capture) {
      for (final Barcode code in capture.barcodes) {
        if (code.rawValue case final text?) seen(text);
      }
    },
  );
}
