import 'dart:async';

import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import 'package:wellfed/platform/join_ports.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/join.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/ask_text.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/replies.dart';

enum _Step { scan, review, joining, joined }

/// Join a feeder with someone's invite (4.1 to 4.3). An invite works once, so
/// a join whose answer never came is not sent again: the screen says so and
/// asks for a new one.
class JoinScreen extends StatefulWidget {
  const JoinScreen({super.key, required this.book, required this.ports, this.scanner = _camera});

  final FeederBook book;
  final JoinPorts ports;

  /// What reads a code, handing on each text it sees.
  final Widget Function(void Function(String)) scanner;

  static Widget _camera(void Function(String) seen) => MobileScanner(
    onDetect: (capture) {
      for (final Barcode code in capture.barcodes) {
        if (code.rawValue case final text?) seen(text);
      }
    },
  );

  @override
  State<JoinScreen> createState() => _JoinScreenState();
}

class _JoinScreenState extends State<JoinScreen> {
  static const _findWait = Duration(seconds: 12);

  _Step _step = _Step.scan;
  String? _invite;
  String _identity = '';
  String? _address;
  bool _finding = false;
  String? _badText;

  /// Said on the review step: why this join came to nothing.
  String? _problem;

  /// The invite may be spent or was refused: only a new one can go on.
  bool _spent = false;
  ClientRole? _role;

  late final _feederName = TextEditingController(text: 'Feeder');
  late final _phoneName = TextEditingController(text: widget.book.label);

  @override
  void dispose() {
    _feederName.dispose();
    _phoneName.dispose();
    super.dispose();
  }

  void _seen(String text) {
    if (_step != _Step.scan) return;
    final String? identity = widget.ports.inviteFeeder(text);
    if (identity == null) {
      setState(() => _badText = "That isn't a Wellfed invite.");
      return;
    }
    setState(() {
      _invite = text;
      _identity = identity;
      _badText = null;
      _step = _Step.review;
    });
    unawaited(_find());
  }

  Future<void> _find() async {
    setState(() => _finding = true);
    final String? address = await widget.ports.locate(_identity, _findWait);
    if (!mounted) return;
    setState(() {
      _address = address;
      _finding = false;
    });
  }

  Future<void> _paste() async {
    final String? text = await askText(context, title: 'Paste an invite link', label: 'Invite', action: 'Use it');
    if (text != null && mounted) _seen(text);
  }

  Future<void> _join() async {
    final String? invite = _invite;
    final String? address = _address;
    if (invite == null || address == null) return;

    setState(() {
      _step = _Step.joining;
      _problem = null;
    });
    final String label = _phoneName.text.trim();
    try {
      final JoinAnswer answer = await widget.ports.join(invite, label, address);
      switch (answer) {
        case JoinAnswer_Joined(:final role):
          await widget.book.setLabel(label);
          await widget.book.remember(KnownFeeder(identity: _identity, name: _feederName.text.trim(), address: address));
          if (mounted) {
            setState(() {
              _role = role;
              _step = _Step.joined;
            });
          }
        case JoinAnswer_Refused(:final refusal):
          _stop(refusalWords(refusal), spent: true);
      }
    } on BridgeError catch (error) {
      _stop(switch (error.kind) {
        ErrorKind.connect => "Couldn't reach the feeder. Nothing was sent.",
        ErrorKind.closed =>
          'The feeder turned this invite down: it has expired or been used. '
              'Ask for a new one.',
        ErrorKind.label => 'A phone name is 1 to 16 bytes.',
        ErrorKind.store => 'Joined, but this phone could not save it.',
        ErrorKind.interrupted =>
          'The connection broke, so you may or may not have joined. Wellfed '
              "won't send this invite again. Ask for a new one.",
        ErrorKind.timeout =>
          "No answer came, so you may or may not have joined. Wellfed won't "
              'send this invite again. Ask for a new one.',
        _ => 'That did not work: ${error.message}',
      }, spent: error.kind != ErrorKind.connect && error.kind != ErrorKind.label);
    }
  }

  void _stop(String problem, {required bool spent}) {
    if (!mounted) return;
    setState(() {
      _problem = problem;
      _spent = spent;
      _step = _Step.review;
    });
  }

  @override
  Widget build(BuildContext context) => switch (_step) {
    _Step.scan => _scan(context),
    _Step.review || _Step.joining => _review(context),
    _Step.joined => _joined(context),
  };

  Widget _scan(BuildContext context) {
    final ThemeTokens c = context.colors;
    return StepScaffold(
      appBarTitle: 'Join with an invite',
      body:
          'Ask an owner to open People › Invite someone on their phone, then '
          'point this camera at the code.',
      content: [
        ClipRRect(
          borderRadius: BorderRadius.circular(16),
          child: SizedBox(height: 300, child: widget.scanner(_seen)),
        ),
        if (_badText case final bad?)
          Padding(
            padding: const EdgeInsets.only(top: Space.s3),
            child: Text(bad, style: TextStyle(color: c.fault)),
          ),
      ],
      actions: [OutlinedButton(onPressed: () => unawaited(_paste()), child: const Text('Paste an invite link'))],
    );
  }

  Widget _review(BuildContext context) {
    final joining = _step == _Step.joining;
    final where = _finding
        ? 'Looking…'
        : _address == null
        ? 'Not found'
        : 'Found';
    return StepScaffold(
      title: "You're invited to a feeder",
      content: [
        FactList([('On your Wi-Fi', Text(where))]),
        if (_address == null && !_finding) ...[
          const SizedBox(height: Space.s3),
          const Notice(
            tone: Tone.neutral,
            glyph: Glyph.wifi,
            text: 'Join the same Wi-Fi as the feeder, then look again.',
          ),
          const SizedBox(height: Space.s2),
          OutlinedButton(onPressed: () => unawaited(_find()), child: const Text('Look again')),
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
        if (_problem case final problem?) ...[
          const SizedBox(height: Space.s4),
          Notice(tone: Tone.fault, glyph: Glyph.warning, text: problem),
        ],
      ],
      actions: [
        FilledButton(
          onPressed:
              joining || _spent || _address == null || _feederName.text.trim().isEmpty || _phoneName.text.trim().isEmpty
              ? null
              : () => unawaited(_join()),
          child: Text(joining ? 'Joining…' : 'Join ${_feederName.text.trim()}'),
        ),
      ],
    );
  }

  Widget _joined(BuildContext context) {
    final String name = _feederName.text.trim();
    final owner = _role == ClientRole.owner;
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
  }
}
