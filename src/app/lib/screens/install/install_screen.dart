import 'dart:async';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/install/install_before.dart';
import 'package:wellfed/screens/install/install_progress.dart';
import 'package:wellfed/screens/install/install_stopped.dart';
import 'package:wellfed/screens/setup/name_step.dart';
import 'package:wellfed/screens/setup/setup_screen.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

/// Installing Wellfed on a feeder that runs stock, one screen per stage of
/// its [InstallFlow].
class InstallScreen extends StatefulWidget {
  const InstallScreen({super.key, required this.newFlow});

  /// The controller of this screen's one install.
  final InstallFlow Function() newFlow;

  @override
  State<InstallScreen> createState() => _InstallScreenState();
}

class _InstallScreenState extends State<InstallScreen> {
  late final InstallFlow _flow = widget.newFlow();

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
        content: Text(
          'This $deviceName stops serving the feeder. The install request is '
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
      child: switch (_flow.state) {
        InstallLoading() => const Scaffold(body: Center(child: CircularProgressIndicator())),
        InstallUnavailable() => const InstallUnavailablePage(),
        final InstallBefore state => InstallBeforePage(flow: _flow, state: state),
        InstallNaming() => NameStep(
          initial: _flow.label,
          action: 'Continue',
          suggest: _flow.phoneModel,
          onBack: () => _flow.back(InstallStage.before),
          onDone: (label, _) => _flow.setLabel(label),
        ),
        InstallWifi() => WifiStep(
          initial: null,
          currentNetwork: _flow.currentNetwork,
          scanNetworks: _flow.scanNetworks,
          stepLabel: 'Steps 1–2 of 5',
          progress: 0.1,
          mayKeep: false,
          onBack: () => _flow.back(InstallStage.before),
          onDone: _flow.setHome,
        ),
        final InstallReset state => InstallResetPage(flow: _flow, state: state),
        final InstallHandover state => InstallHandoverPage(flow: _flow, state: state, onLeave: _leave),
        final InstallConfirm state => InstallConfirmPage(flow: _flow, state: state, onLeave: _leave),
        InstallWhich() => InstallWhichFeederPage(flow: _flow),
        final InstallSending state => InstallSendingPage(flow: _flow, state: state),
        final InstallClaim state => InstallClaimPage(flow: _flow, state: state),
        final InstallFinishing state => InstallFinishingPage(flow: _flow, state: state),
        final InstallDone state => InstallDonePage(flow: _flow, state: state),
        final InstallStopped state => InstallStoppedPage(flow: _flow, state: state),
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

/// The install refused on a device that cannot join the feeder's own Wi-Fi.
class InstallUnavailablePage extends StatelessWidget {
  const InstallUnavailablePage({super.key});

  @override
  Widget build(BuildContext context) => StepScaffold(
    leading: Leading.close,
    title: 'Install Wellfed',
    content: [
      Notice(
        tone: Tone.fault,
        lead: 'This $deviceName cannot install Wellfed.',
        text:
            "The install joins the feeder's own Wi-Fi, which only the Android "
            'app can do. Install from an Android phone, then recover the feeder '
            'here or join it with an invite.',
      ),
    ],
  );
}
