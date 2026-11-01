import 'dart:async';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/install/install_screen.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/code_tiles.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

/// 2.3: this phone joins the feeder's network, sends the Wi-Fi once and comes
/// back, then waits for the feeder to reach it.
class InstallHandover extends StatelessWidget {
  const InstallHandover({super.key, required this.flow, required this.onLeave});

  final InstallFlow flow;
  final Future<void> Function() onLeave;

  @override
  Widget build(BuildContext context) {
    final String? stateText = switch (flow.handover) {
      Handover.starting => 'Getting ready…',
      Handover.join || Handover.joining => null,
      Handover.sending => 'Sending your Wi-Fi to the feeder…',
      Handover.waiting => 'Waiting for the feeder to join your Wi-Fi…',
    };
    return StepScaffold(
      stepLabel: 'Step 2 of 5',
      progress: 0.35,
      leading: Leading.close,
      onLeading: () => unawaited(onLeave()),
      title: 'Tell the feeder how to reach this phone',
      body:
          "This phone joins the feeder's setup network for a few seconds, "
          'sends your Wi-Fi once, then comes back to your network.',
      content: [
        const Notice(
          tone: Tone.neutral,
          lead: 'Your password goes unencrypted.',
          text:
              "The feeder's setup network has no password, so someone close "
              'by could read it during those few seconds. The feeder also '
              'needs a 2.4 GHz Wi-Fi.',
        ),
        if (flow.joinFailed) ...[
          const SizedBox(height: Space.s3),
          const Notice(
            tone: Tone.warning,
            lead: "This phone didn't join the feeder's network.",
            text:
                'Its name starts with PL-AF103. It can take a minute to appear '
                "after the reset. If this phone set it up in the vendor's app, "
                "forget that network in the phone's Wi-Fi settings, then try "
                'again. If it never appears, hold Reset again.',
          ),
        ],
        if (flow.restartAgain || flow.waitReason != null) ...[
          const SizedBox(height: Space.s4),
          _StillWaiting(flow: flow),
        ] else if (stateText != null) ...[
          const SizedBox(height: Space.s4),
          Searching(stateText),
        ],
      ],
      actions: [
        if (flow.handover == Handover.join || flow.handover == Handover.joining)
          FilledButton(
            onPressed: flow.handover == Handover.join ? () => unawaited(flow.joinFeeder()) : null,
            child: const Text("Join the feeder's network"),
          ),
      ],
    );
  }
}

/// Why stock has not moved on, and the restart its lost reports need.
class _StillWaiting extends StatelessWidget {
  const _StillWaiting({required this.flow});

  final InstallFlow flow;

  @override
  Widget build(BuildContext context) {
    final String text = switch (flow.waitReason) {
      WaitReason.notOnAdapter => 'A feeder connected, but it is on batteries. Plug it in.',
      WaitReason.noVersion || WaitReason.offline => 'A feeder is connecting…',
      _ => 'Waiting for the feeder to connect…',
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (flow.restartAgain) ...[
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

/// 2.4: trust on first use. The owner compares what connected with the
/// label and the router's list before anything is sent to it.
class InstallConfirm extends StatelessWidget {
  const InstallConfirm({super.key, required this.flow, required this.onLeave});

  final InstallFlow flow;
  final Future<void> Function() onLeave;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final mono = TextStyle(fontFamily: Fonts.mono, fontSize: 15, color: c.text);
    return StepScaffold(
      stepLabel: 'Step 3 of 5',
      progress: 0.5,
      leading: Leading.close,
      onLeading: () => unawaited(onLeave()),
      title: 'A feeder connected. Is it yours?',
      body:
          'Compare the device ID with the label under the feeder, and the '
          "address with your router's list of devices.",
      content: [
        FactList([
          ('Device ID', Text(flow.device ?? '', style: mono)),
          ('Address', Text(flow.deviceAddress ?? '', style: mono)),
        ]),
        const SizedBox(height: Space.s4),
        const InstallFact('PLAF103'),
        const InstallFact('Vendor firmware 1.1.15'),
        const InstallFact('On its mains adapter'),
      ],
      actions: [
        FilledButton(onPressed: () => flow.recognize(yes: true), child: const Text('Yes, install Wellfed')),
        TextButton(onPressed: () => flow.recognize(yes: false), child: const Text("That's not my feeder")),
      ],
    );
  }
}

/// A screen with the feeder on it and no way back: the run goes on by
/// itself, and the notification speaks for it when the phone is elsewhere.
class _FeederPage extends StatelessWidget {
  const _FeederPage({required this.title, required this.children});

  final String title;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Scaffold(
    body: SafeArea(
      child: ListView(
        padding: const EdgeInsets.fromLTRB(28, 56, 28, 28),
        children: [
          const Center(child: FeederImage(size: 180)),
          const SizedBox(height: 28),
          Headline(title, size: 24, align: TextAlign.center),
          const SizedBox(height: 28),
          ...children,
        ],
      ),
    ),
  );
}

/// 2.5: stock downloads the replacement from this phone.
class InstallSending extends StatelessWidget {
  const InstallSending({super.key, required this.flow});

  final InstallFlow flow;

  String _status() {
    if (flow.restarting) return 'Restarting into Wellfed';
    final Duration? left = flow.timeLeft;
    if (left == null) return '${flow.percent}%';
    return '${flow.percent}% · about ${left.inSeconds + 1} s left';
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return _FeederPage(
      title: 'Installing Wellfed',
      children: [
        Row(
          children: [
            Expanded(
              child: Text(
                flow.restarting ? 'The feeder is restarting' : 'Sending firmware',
                style: TextStyle(fontSize: 15, fontWeight: Fonts.semibold, color: c.text),
              ),
            ),
            Flexible(
              child: Text(
                _status(),
                style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
              ),
            ),
          ],
        ),
        const SizedBox(height: Space.s2),
        Bar(value: flow.restarting ? 1 : flow.percent / 100),
        const SizedBox(height: Space.s5),
        if (flow.holdRefused) ...[
          const Notice(
            tone: Tone.fault,
            lead: "Android won't keep Wellfed running in the background.",
            text: 'Keep it on the screen until this finishes.',
          ),
          const SizedBox(height: Space.s3),
        ],
        const Notice(
          tone: Tone.warning,
          lead: 'Keep Wellfed open.',
          text:
              'This phone is serving the firmware, and closing the app stops '
              "it. Don't unplug the feeder.",
        ),
        WaitWords(flow.waitWords, progress: flow.percent),
      ],
    );
  }
}

/// 2.6: this phone becomes the replacement's owner. A consumer image waits
/// on Bluetooth by itself; a bench image needs its recovery window opened.
/// Either way the code is the feeder's proof, as in setup.
class InstallClaim extends StatelessWidget {
  const InstallClaim({super.key, required this.flow});

  final InstallFlow flow;

  @override
  Widget build(BuildContext context) => StepScaffold(
    stepLabel: 'Step 4 of 5',
    progress: 0.75,
    leading: Leading.none,
    title: 'Wellfed is running. Now claim it.',
    content: [
      if (flow.windowAsked)
        const NumberedSteps([
          'Hold the **Lock** button, then also the **Dispense** button, for 5 seconds. The lock light blinks fast.',
        ])
      else if (flow.code.isEmpty) ...[
        if (flow.settingUpAgain) ...[
          const NumberedSteps([
            'If the lock light is on, hold **Lock** for 5 seconds to unlock the panel.',
            'Hold **Lock** and **Mic** together for 5 seconds.',
            'The Wi-Fi light blinks slowly, about once a second. Setup mode lasts 15 minutes.',
          ]),
          const SizedBox(height: Space.s4),
        ],
        if (flow.setupHint && !flow.settingUpAgain) ...[
          const Notice(
            tone: Tone.warning,
            lead: 'Hold Lock + Mic for 5 s on the feeder.',
            text:
                'If the lock light is on, hold Lock for 5 seconds to unlock '
                'the panel first.',
          ),
          const SizedBox(height: Space.s4),
        ],
        Searching(
          flow.searchingBluetooth || flow.settingUpAgain
              ? 'Looking for the feeder over Bluetooth…'
              : 'Looking for the feeder on your Wi-Fi…',
        ),
        WaitWords(flow.waitWords),
      ] else ...[
        Text('Press these buttons once each, in order:', style: bodyStyle(context)),
        const SizedBox(height: Space.s3),
        CodeTiles(flow.code),
        const SizedBox(height: 18),
        const RingLegend(),
        const SizedBox(height: 18),
        CodeWindow(deadline: flow.deadline),
        const SizedBox(height: Space.s4),
        const Searching('Waiting for the feeder to confirm…'),
      ],
      if (flow.pressDispense) ...[
        const SizedBox(height: Space.s4),
        const Notice(
          tone: Tone.warning,
          lead: 'Press Dispense once.',
          text: "The feeder takes the install's update key with that press.",
        ),
      ],
    ],
    actions: [
      if (flow.windowAsked)
        FilledButton(onPressed: flow.windowOpened, child: const Text('The lock light is blinking fast')),
    ],
  );
}

/// One line of the checklist.
class _Step extends StatelessWidget {
  const _Step(this.text, this.state, {this.fraction});

  final String text;
  final CheckState state;

  /// How far a running step is, when it knows.
  final double? fraction;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final running = state == CheckState.running;
    final int? percent = fraction == null ? null : (fraction! * 100).round();
    return Padding(
      padding: const EdgeInsets.only(bottom: Space.s3),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              SizedBox.square(
                dimension: 22,
                child: switch (state) {
                  CheckState.done => StrokeIcon(Glyph.check, size: 20, color: c.ok, weight: 2.2),
                  CheckState.running => CircularProgressIndicator(
                    strokeWidth: 2.5,
                    color: c.accent,
                    backgroundColor: c.sunken,
                  ),
                  CheckState.waiting => Center(
                    child: Container(
                      width: 8,
                      height: 8,
                      decoration: BoxDecoration(color: c.lineStrong, shape: BoxShape.circle),
                    ),
                  ),
                },
              ),
              const SizedBox(width: Space.s3),
              Expanded(
                child: Text(
                  text,
                  style: TextStyle(
                    fontSize: 15,
                    fontWeight: running ? Fonts.semibold : null,
                    color: state == CheckState.waiting ? c.textMuted : c.text,
                  ),
                ),
              ),
              if (running && percent != null)
                Text(
                  '$percent%',
                  style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                ),
            ],
          ),
          if (running && fraction != null) ...[const SizedBox(height: Space.s2), Bar(value: fraction!)],
        ],
      ),
    );
  }
}

/// 2.7: the backup, the promotion and the confirmation, one line each.
class InstallFinishing extends StatelessWidget {
  const InstallFinishing({super.key, required this.flow});

  final InstallFlow flow;

  @override
  Widget build(BuildContext context) {
    final String title = switch (flow.promoting) {
      CheckState.waiting => 'Backing up the original firmware',
      _ => 'Making Wellfed permanent',
    };
    return StepScaffold(
      stepLabel: 'Step 5 of 5',
      progress: 1,
      leading: Leading.none,
      title: title,
      content: [
        const _Step('This phone is its owner', CheckState.done),
        if (flow.joining != CheckState.waiting) _Step('Finding it on your network', flow.joining),
        _Step('Copying the original firmware', flow.copying, fraction: flow.copied),
        _Step('Check the copy matches', flow.checked),
        _Step('Make Wellfed permanent · about 4 min', flow.promoting, fraction: flow.written),
        _Step('Restart and confirm', flow.confirming),
        const SizedBox(height: Space.s2),
        Text(
          'Wellfed only becomes permanent once the backup checks out. Until '
          'then, the original firmware is still on the feeder.',
          style: bodyStyle(context),
        ),
        const SizedBox(height: Space.s4),
        const Notice(
          tone: Tone.neutral,
          lead: 'Keep the feeder plugged in.',
          text: 'If this phone loses it, Wellfed picks up where it stopped.',
        ),
        WaitWords(
          flow.waitWords,
          progress: (
            flow.joining,
            flow.copied,
            flow.copying,
            flow.checked,
            flow.written,
            flow.promoting,
            flow.confirming,
          ),
        ),
      ],
    );
  }
}

/// 2.8: done. Names the feeder.
class InstallDone extends StatefulWidget {
  const InstallDone({super.key, required this.flow});

  final InstallFlow flow;

  @override
  State<InstallDone> createState() => _DoneState();
}

class _DoneState extends State<InstallDone> {
  final _name = TextEditingController(text: InstallFlow.defaultFeederName);
  String? _shareFailed;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _go() async {
    final String name = _name.text.trim();
    await widget.flow.finish(name.isEmpty ? InstallFlow.defaultFeederName : name);
    if (mounted) context.go('/');
  }

  Future<void> _save() async {
    try {
      await widget.flow.saveBackup();
    } catch (error) {
      if (mounted) setState(() => _shareFailed = '$error');
    }
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final InstallFlow flow = widget.flow;
    return Scaffold(
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(28, Space.s7, 28, 28),
          children: [
            Center(
              child: Stack(
                children: [
                  const FeederImage(size: 250),
                  Positioned(right: 0, top: 0, child: StrokeIcon(Glyph.check, size: 56, color: c.ok, weight: 2.2)),
                ],
              ),
            ),
            const Headline('Your feeder runs Wellfed.', size: 32),
            const SizedBox(height: Space.s2),
            Text('Set up its meals here next.', style: bodyStyle(context)),
            const SizedBox(height: Space.s5),
            LabeledField(
              label: 'Name it',
              child: TextField(controller: _name, textCapitalization: TextCapitalization.words),
            ),
            const SizedBox(height: Space.s4),
            const SizedBox(height: Space.s5),
            FilledButton(onPressed: () => unawaited(_go()), child: const Text('Go to the feeder')),
            if (flow.backupPath != null) ...[
              const SizedBox(height: Space.s2),
              TextButton(onPressed: () => unawaited(_save()), child: const Text('Save the backup somewhere else')),
            ],
            if (_shareFailed != null)
              Notice(tone: Tone.fault, lead: "The backup wasn't saved elsewhere.", text: _shareFailed!),
          ],
        ),
      ),
    );
  }
}
