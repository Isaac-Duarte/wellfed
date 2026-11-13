import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/layout.dart';

/// 2.3: this device joins the feeder's network, sends the Wi-Fi once and
/// comes back, then waits for the feeder to reach it. A phone does each switch
/// itself; a computer's owner does them in its Wi-Fi menu, guided here.
class InstallHandoverPage extends StatelessWidget {
  const InstallHandoverPage({super.key, required this.flow, required this.state, required this.onLeave});

  final InstallFlow flow;
  final InstallHandover state;
  final Future<void> Function() onLeave;

  @override
  Widget build(BuildContext context) {
    final bool byHand = state.join == FeederJoin.byHand;
    final String? stateText = switch (state.handover) {
      Handover.starting => 'Getting ready…',
      Handover.join || Handover.returning => null,
      Handover.joining => byHand ? "Waiting for this computer to join the feeder's network…" : null,
      Handover.sending => 'Sending your Wi-Fi to the feeder…',
      Handover.waiting => 'Waiting for the feeder to join your Wi-Fi…',
    };
    final bool joinStep = state.handover == Handover.join || state.handover == Handover.joining;
    return StepScaffold(
      stepLabel: 'Step 2 of 5',
      progress: 0.35,
      leading: Leading.close,
      onLeading: () => unawaited(onLeave()),
      title: 'Tell the feeder how to reach this $deviceName',
      body: byHand
          ? "Switch this computer to the feeder's setup network for a few seconds. "
                'It sends your Wi-Fi once, then you switch straight back.'
          : "This $deviceName joins the feeder's setup network for a few seconds, "
                'sends your Wi-Fi once, then comes back to your network.',
      content: [
        if (byHand && state.handover == Handover.returning)
          _GoBack(state: state)
        else ...[
          if (byHand && joinStep) ...[_JoinByHand(state: state), const SizedBox(height: Space.s4)],
          const Notice(
            tone: Tone.neutral,
            lead: 'Your password goes unencrypted.',
            text:
                "The feeder's setup network has no password, so someone close "
                'by could read it during those few seconds. The feeder also '
                'needs a 2.4 GHz Wi-Fi.',
          ),
        ],
        if (state.joinFailed) ...[const SizedBox(height: Space.s3), _NotJoined(state: state)],
        if (state.restartAgain || state.waitReason != null) ...[
          const SizedBox(height: Space.s4),
          _StillWaiting(state: state),
        ] else if (stateText != null) ...[
          const SizedBox(height: Space.s4),
          Searching(stateText),
        ],
      ],
      actions: [
        if (joinStep && !(byHand && state.handover == Handover.joining))
          FilledButton(
            onPressed: state.handover == Handover.join ? () => unawaited(flow.joinFeeder()) : null,
            child: Text(byHand ? 'Look for it again' : "Join the feeder's network"),
          ),
        if (byHand && state.handover == Handover.joining)
          OutlinedButton(onPressed: flow.stopJoining, child: const Text('Stop waiting')),
      ],
    );
  }
}

/// Why this device was not counted as on the feeder's network, and what to
/// change. A computer refuses another 192.168.1.x network outright, since the
/// Wi-Fi password would go to its router.
class _NotJoined extends StatelessWidget {
  const _NotJoined({required this.state});

  final InstallHandover state;

  @override
  Widget build(BuildContext context) {
    final (String lead, String text) = switch (state.joinProblem) {
      FeederJoinProblem.otherNetwork => (
        'This computer is already on another 192.168.1.x network.',
        "That can be a second adapter, a guest network or another router, and your Wi-Fi password mustn't go "
            "there. Disconnect it, then look again. If it's the feeder's network, switch back to your home "
            'network first.',
      ),
      FeederJoinProblem.nameUnreadable => (
        "This computer didn't say which Wi-Fi it joined.",
        '${_nameHelp()} Your Wi-Fi password goes only to a network named PL-AF103-…, so nothing was sent.',
      ),
      FeederJoinProblem.alreadyOnFeeder => (
        "This computer was already on the feeder's network.",
        'The feeder would look for it at the wrong address. Switch back to your home network, then close this '
            'install and start again. The feeder keeps its setup network open.',
      ),
      FeederJoinProblem.vpn => (
        "The feeder can't reach this computer's address.",
        "It's a VPN's, or another address off your home network. Turn off your VPN, then look again.",
      ),
      FeederJoinProblem.notSeen when state.join == FeederJoin.byHand => (
        "This computer didn't join the feeder's network.",
        'Its name starts with PL-AF103. It can take a minute to appear after the reset. If a VPN is on, turn it '
            'off: it can hide the feeder. If the network never appears, hold Reset again.',
      ),
      _ => (
        "This $deviceName didn't join the feeder's network.",
        'Its name starts with PL-AF103. It can take a minute to appear after the reset. If this $deviceName set '
            "it up in the vendor's app, forget that network in the $deviceName's Wi-Fi settings, then try again. "
            'If it never appears, hold Reset again.',
      ),
    };
    return Notice(tone: Tone.warning, lead: lead, text: text);
  }
}

/// How to let Wellfed read the Wi-Fi's name, on this computer's system.
String _nameHelp() => switch (defaultTargetPlatform) {
  TargetPlatform.macOS =>
    'macOS names the Wi-Fi only to apps allowed your location. Allow Wellfed in System Settings › Privacy & '
        'Security › Location Services, then look again.',
  TargetPlatform.windows =>
    'Windows names the Wi-Fi only to apps allowed your location. Turn on Location, and Let desktop apps access '
        'your location, in Settings › Privacy & security › Location, then look again.',
  _ =>
    "Wellfed asks NetworkManager (nmcli) or iw, and needs the feeder's network joined over Wi-Fi. Install either, "
        'or join it with the Wi-Fi rather than a cable, then look again.',
};

/// What a computer's owner does, in order, and what makes it quick: a cable
/// keeps the home network up throughout.
class _JoinByHand extends StatelessWidget {
  const _JoinByHand({required this.state});

  final InstallHandover state;

  @override
  Widget build(BuildContext context) {
    final back =
        "Then switch straight back to **${state.homeNetwork ?? 'your home network'}**. The feeder gives up if it "
        "can't reach this computer within about 20 seconds of joining your Wi-Fi.";
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        NumberedSteps([
          "Open this computer's Wi-Fi menu and pick the network whose name starts with **PL-AF103-**. It has no password.",
          'Wellfed sees the switch and sends your Wi-Fi to the feeder.',
          back,
        ]),
        const SizedBox(height: Space.s4),
        if (state.homeSharesSetupAddresses)
          const Notice(
            tone: Tone.warning,
            lead: "Your home network uses the feeder's addresses.",
            text:
                'Both use 192.168.1.x, so this computer only reaches the feeder '
                'once it is off your home network. Unplug its cable too, if it has '
                'one, and plug it back in straight after.',
          )
        else
          const Notice(
            tone: Tone.neutral,
            lead: 'Quickest with a cable.',
            text:
                'If this computer has a wired connection, or a second Wi-Fi adapter, '
                "keep your home network on that and join the feeder's network with "
                'the Wi-Fi. The feeder then finds this computer with no switching '
                'back.',
          ),
      ],
    );
  }
}

/// The frame went out: the owner switches back now, as stock's wait for this
/// computer is short.
class _GoBack extends StatelessWidget {
  const _GoBack({required this.state});

  final InstallHandover state;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      Notice(
        tone: Tone.warning,
        lead: 'Switch back to ${state.homeNetwork ?? 'your home network'} now.',
        text:
            'The feeder is joining your Wi-Fi, and looks for this computer there '
            'within about 20 seconds.',
      ),
      const SizedBox(height: Space.s4),
      const Searching('Waiting for this computer to be back on your network…'),
    ],
  );
}

/// Why stock has not moved on, and the restart its lost reports need.
class _StillWaiting extends StatelessWidget {
  const _StillWaiting({required this.state});

  final InstallHandover state;

  @override
  Widget build(BuildContext context) {
    final String text = switch (state.waitReason) {
      WaitReason.notOnAdapter => 'A feeder connected, but it is on batteries. Plug it in.',
      WaitReason.noVersion || WaitReason.offline => 'A feeder is connecting…',
      _ => 'Waiting for the feeder to connect…',
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (state.restartAgain) ...[
          const Notice(
            tone: Tone.warning,
            lead: 'Restart the feeder once more:',
            text:
                'unplug it for 10 seconds (take the batteries out too, if '
                'fitted) and plug it back in.',
          ),
          const SizedBox(height: Space.s4),
        ],
        Searching(text),
      ],
    );
  }
}
