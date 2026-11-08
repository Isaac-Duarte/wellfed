import 'dart:async';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/install/install_screen.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/code_tiles.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

/// 2.3: this phone joins the feeder's network, sends the Wi-Fi once and comes
/// back, then waits for the feeder to reach it.
class InstallHandoverPage extends StatelessWidget {
  const InstallHandoverPage({super.key, required this.flow, required this.state, required this.onLeave});

  final InstallFlow flow;
  final InstallHandover state;
  final Future<void> Function() onLeave;

  @override
  Widget build(BuildContext context) {
    final String? stateText = switch (state.handover) {
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
      title: 'Tell the feeder how to reach this $deviceName',
      body:
          "This $deviceName joins the feeder's setup network for a few seconds, "
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
        if (state.joinFailed) ...[
          const SizedBox(height: Space.s3),
          Notice(
            tone: Tone.warning,
            lead: "This $deviceName didn't join the feeder's network.",
            text:
                'Its name starts with PL-AF103. It can take a minute to appear '
                "after the reset. If this $deviceName set it up in the vendor's app, "
                "forget that network in the $deviceName's Wi-Fi settings, then try "
                'again. If it never appears, hold Reset again.',
          ),
        ],
        if (state.restartAgain || state.waitReason != null) ...[
          const SizedBox(height: Space.s4),
          _StillWaiting(state: state),
        ] else if (stateText != null) ...[
          const SizedBox(height: Space.s4),
          Searching(stateText),
        ],
      ],
      actions: [
        if (state.handover == Handover.join || state.handover == Handover.joining)
          FilledButton(
            onPressed: state.handover == Handover.join ? () => unawaited(flow.joinFeeder()) : null,
            child: const Text("Join the feeder's network"),
          ),
      ],
    );
  }
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

/// 2.4: trust on first use. The owner compares what connected with the
/// label and the router's list before anything is sent to it.
class InstallConfirmPage extends StatelessWidget {
  const InstallConfirmPage({super.key, required this.flow, required this.state, required this.onLeave});

  final InstallFlow flow;
  final InstallConfirm state;
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
          ('Device ID', Text(state.device ?? '', style: mono)),
          ('Address', Text(state.deviceAddress ?? '', style: mono)),
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

/// 2.5: stock downloads the replacement from this phone, then restarts into
/// it. There is no way back from here: the run goes on by itself, and the
/// notification speaks for it when the phone is elsewhere.
class InstallSendingPage extends StatelessWidget {
  const InstallSendingPage({super.key, required this.flow, required this.state});

  final InstallFlow flow;
  final InstallSending state;

  @override
  Widget build(BuildContext context) {
    final Duration? left = state.restarting ? null : flow.timeLeft;
    return StepScaffold(
      stepLabel: 'Step 3 of 5',
      progress: 0.6,
      leading: Leading.none,
      title: 'Installing Wellfed',
      body: 'The feeder downloads Wellfed from this $deviceName, then restarts into it.',
      content: [
        _Step(
          'Sending firmware',
          state.restarting ? CheckState.done : CheckState.running,
          fraction: state.percent / 100,
          detail: left == null ? null : 'About ${left.inSeconds + 1} s left',
        ),
        _Step('Restarting into Wellfed', state.restarting ? CheckState.running : CheckState.waiting),
        const SizedBox(height: Space.s2),
        if (state.holdRefused) ...[
          const Notice(
            tone: Tone.fault,
            lead: "Android won't keep Wellfed running in the background.",
            text: 'Keep it on the screen until this finishes.',
          ),
          const SizedBox(height: Space.s3),
        ],
        Notice(
          tone: Tone.warning,
          lead: 'Keep Wellfed open.',
          text:
              'This $deviceName is serving the firmware, and closing the app stops '
              "it. Don't unplug the feeder.",
        ),
        WaitWords(flow.waitWords, progress: state.percent),
      ],
    );
  }
}

/// 2.6: this phone becomes the replacement's owner. A consumer image waits
/// on Bluetooth by itself; a bench image needs its recovery window opened.
/// Either way the code is the feeder's proof, as in setup.
class InstallClaimPage extends StatelessWidget {
  const InstallClaimPage({super.key, required this.flow, required this.state});

  final InstallFlow flow;
  final InstallClaim state;

  @override
  Widget build(BuildContext context) => StepScaffold(
    stepLabel: 'Step 4 of 5',
    progress: 0.75,
    leading: Leading.none,
    title: 'Wellfed is running. Now claim it.',
    content: [
      if (state.windowAsked)
        const NumberedSteps([
          'Hold the **Lock** button, then also the **Dispense** button, for 5 seconds. The lock light blinks fast.',
        ])
      else if (state.code.isEmpty) ...[
        if (state.settingUpAgain) ...[
          const NumberedSteps([
            'If the lock light is on, hold **Lock** for 5 seconds to unlock the panel.',
            'Hold **Lock** and **Mic** together for 5 seconds.',
            'The Wi-Fi light blinks slowly, about once a second. Setup mode lasts 15 minutes.',
          ]),
          const SizedBox(height: Space.s4),
        ],
        if (state.setupHint && !state.settingUpAgain) ...[
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
          state.searchingBluetooth || state.settingUpAgain
              ? 'Looking for the feeder over Bluetooth…'
              : 'Looking for the feeder on your Wi-Fi…',
        ),
        WaitWords(flow.waitWords),
      ] else ...[
        Text('Press these buttons once each, in order:', style: bodyStyle(context)),
        const SizedBox(height: Space.s3),
        CodeTiles(state.code),
        const SizedBox(height: 18),
        const RingLegend(),
        const SizedBox(height: 18),
        CodeWindow(deadline: state.deadline),
        const SizedBox(height: Space.s4),
        const Searching('Waiting for the feeder to confirm…'),
      ],
      if (state.pressDispense) ...[
        const SizedBox(height: Space.s4),
        const Notice(
          tone: Tone.warning,
          lead: 'Press Dispense once.',
          text: "The feeder takes the install's update key with that press.",
        ),
      ],
    ],
    actions: [
      if (state.windowAsked)
        FilledButton(onPressed: flow.windowOpened, child: const Text('The lock light is blinking fast')),
    ],
  );
}

/// One line of the checklist.
class _Step extends StatelessWidget {
  const _Step(this.text, this.state, {this.fraction, this.detail});

  final String text;
  final CheckState state;

  /// How far a running step is, when it knows.
  final double? fraction;

  /// A muted line under a running step, such as the time left.
  final String? detail;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final running = state == CheckState.running;
    final double? part = fraction;
    final int? percent = part == null ? null : (part * 100).round();
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
          if (running && part != null) ...[const SizedBox(height: Space.s2), Bar(value: part)],
          if (detail case final detail? when running) ...[
            const SizedBox(height: Space.s1),
            Text(
              detail,
              style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
            ),
          ],
        ],
      ),
    );
  }
}

/// 2.7: the backup, the promotion and the confirmation, one line each.
class InstallFinishingPage extends StatelessWidget {
  const InstallFinishingPage({super.key, required this.flow, required this.state});

  final InstallFlow flow;
  final InstallFinishing state;

  @override
  Widget build(BuildContext context) {
    final String title = switch (state.promoting) {
      CheckState.waiting => 'Backing up the original firmware',
      _ => 'Making Wellfed permanent',
    };
    return StepScaffold(
      stepLabel: 'Step 5 of 5',
      progress: 1,
      leading: Leading.none,
      title: title,
      content: [
        _Step('This $deviceName is its owner', CheckState.done),
        if (state.joining != CheckState.waiting) _Step('Finding it on your network', state.joining),
        _Step('Copying the original firmware', state.copying, fraction: state.copied),
        _Step('Check the copy matches', state.checked),
        _Step('Make Wellfed permanent · about 4 min', state.promoting, fraction: state.written),
        _Step('Restart and confirm', state.confirming),
        const SizedBox(height: Space.s2),
        Text(
          'Wellfed only becomes permanent once the backup checks out. Until '
          'then, the original firmware is still on the feeder.',
          style: bodyStyle(context),
        ),
        const SizedBox(height: Space.s4),
        Notice(
          tone: Tone.neutral,
          lead: 'Keep the feeder plugged in.',
          text: 'If this $deviceName loses it, Wellfed picks up where it stopped.',
        ),
        WaitWords(
          flow.waitWords,
          progress: (
            state.joining,
            state.copied,
            state.copying,
            state.checked,
            state.written,
            state.promoting,
            state.confirming,
          ),
        ),
      ],
    );
  }
}

/// 2.8: done. Names the feeder.
class InstallDonePage extends StatefulWidget {
  const InstallDonePage({super.key, required this.flow, required this.state});

  final InstallFlow flow;
  final InstallDone state;

  @override
  State<InstallDonePage> createState() => _DoneState();
}

class _DoneState extends State<InstallDonePage> {
  final _name = TextEditingController(text: InstallFlow.defaultFeederName);
  String? _shareFailed;

  /// The owner saved the backup elsewhere, or said not now.
  bool _offered = false;

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
    final String? failed = await widget.flow.saveBackup();
    if (mounted) setState(() => failed == null ? _offered = true : _shareFailed = failed);
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
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
            // The one copy of stock is on this device: offered once, plainly.
            if (widget.state.backupPath != null && !_offered) ...[
              Notice(
                tone: Tone.warning,
                lead: 'Save the original firmware somewhere else.',
                text:
                    'Its backup is only on this $deviceName. Keep a copy in your files or cloud storage, so '
                    "you can go back to it if this $deviceName is lost or reset. It's in Settings too.",
              ),
              const SizedBox(height: Space.s2),
              FilledButton.tonal(onPressed: () => unawaited(_save()), child: const Text('Save the backup')),
              TextButton(onPressed: () => setState(() => _offered = true), child: const Text('Not now')),
            ],
            if (_shareFailed case final failed?)
              Notice(tone: Tone.fault, lead: "The backup wasn't saved elsewhere.", text: failed),
            const SizedBox(height: Space.s5),
            FilledButton(onPressed: () => unawaited(_go()), child: const Text('Go to the feeder')),
          ],
        ),
      ),
    );
  }
}
