import 'dart:async';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/screens/install/install_before.dart';
import 'package:wellfed/screens/install/install_progress.dart';
import 'package:wellfed/screens/install/install_stopped.dart';
import 'package:wellfed/screens/setup/name_step.dart';
import 'package:wellfed/screens/setup/setup_screen.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/icons.dart';

/// Installing Wellfed on a feeder that runs stock, one screen per stage of
/// its [InstallFlow].
class InstallScreen extends StatefulWidget {
  const InstallScreen({super.key, required this.book, required this.ports});

  final FeederBook book;
  final InstallPorts ports;

  @override
  State<InstallScreen> createState() => _InstallScreenState();
}

class _InstallScreenState extends State<InstallScreen> {
  late final InstallFlow _flow = InstallFlow(widget.book, widget.ports);

  @override
  void initState() {
    super.initState();
    unawaited(_flow.load());
  }

  @override
  void dispose() {
    _flow.dispose();
    super.dispose();
  }

  /// Leaving with a run on stops it, so the owner says so first. The stop
  /// says what it left unsettled.
  Future<void> _leave() async {
    if (!_flow.running) {
      if (mounted) context.pop();
      return;
    }
    final bool? stop = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Stop the install?'),
        content: const Text(
          'This phone stops serving the feeder. The install request is '
          'never sent a second time.',
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Keep going')),
          TextButton(onPressed: () => Navigator.pop(context, true), child: const Text('Stop')),
        ],
      ),
    );
    if (stop == true) _flow.stopRun();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _flow,
    builder: (context, _) => PopScope(
      canPop: !_flow.running,
      onPopInvokedWithResult: (popped, _) {
        if (!popped) unawaited(_leave());
      },
      child: switch (_flow.stage) {
        InstallStage.loading => const Scaffold(body: Center(child: CircularProgressIndicator())),
        InstallStage.before => InstallBefore(flow: _flow),
        InstallStage.naming => NameStep(
          initial: widget.book.label,
          action: 'Continue',
          onBack: () => _flow.back(InstallStage.before),
          onDone: (label, _) => _flow.setLabel(label),
        ),
        InstallStage.wifi => WifiStep(
          initial: null,
          stepLabel: 'Steps 1–2 of 5',
          progress: 0.1,
          mayKeep: false,
          onBack: () => _flow.back(InstallStage.before),
          onDone: _flow.setHome,
        ),
        InstallStage.reset => InstallReset(flow: _flow),
        InstallStage.handover => InstallHandover(flow: _flow, onLeave: _leave),
        InstallStage.confirm => InstallConfirm(flow: _flow, onLeave: _leave),
        InstallStage.which => InstallWhichFeeder(flow: _flow),
        InstallStage.sending => InstallSending(flow: _flow),
        InstallStage.claim => InstallClaim(flow: _flow),
        InstallStage.finishing => InstallFinishing(flow: _flow),
        InstallStage.done => InstallDone(flow: _flow),
        InstallStage.stopped => InstallStopped(flow: _flow),
      },
    ),
  );
}

/// A row with a check: what is needed, or what is known.
class InstallFact extends StatelessWidget {
  const InstallFact(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Padding(
      padding: const EdgeInsets.only(bottom: 10),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          StrokeIcon(Glyph.check, size: 20, color: c.ok, weight: 2.2),
          const SizedBox(width: 10),
          Expanded(
            child: Text(text, style: TextStyle(fontSize: 15, height: 1.45, color: c.text)),
          ),
        ],
      ),
    );
  }
}
