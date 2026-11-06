import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/restore/restore_stopped.dart';
import 'package:wellfed/state/install/install.dart' show CheckState;
import 'package:wellfed/state/restore/restore.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

/// 12.4: putting the vendor's firmware back from the backup the install kept.
class RestoreScreen extends StatefulWidget {
  const RestoreScreen({super.key, required this.newFlow});

  /// The controller of this screen's one restore.
  final RestoreFlow Function() newFlow;

  @override
  State<RestoreScreen> createState() => _RestoreScreenState();
}

class _RestoreScreenState extends State<RestoreScreen> {
  late final RestoreFlow _flow = widget.newFlow();

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

  @override
  Widget build(BuildContext context) => RestoreView(flow: _flow);
}

/// The flow's screens, whichever stage it is at.
class RestoreView extends StatelessWidget {
  const RestoreView({super.key, required this.flow});

  final RestoreFlow flow;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: flow,
    builder: (context, _) => PopScope(
      canPop: !flow.state.committed,
      child: switch (flow.state) {
        RestoreLoading() => const Scaffold(body: SizedBox.shrink()),
        RestoreUnavailable() => const _Unavailable(),
        final RestoreNoBackup state => _NoBackup(flow, state),
        final RestoreBefore state => _Before(flow, state),
        final RestoreRunning state => _Running(flow, state),
        final RestoreDone state => _Done(flow, state),
        final RestoreUncertain state => RestoreUncertainPage(flow, state),
        final RestoreStopped state => RestoreStoppedPage(flow, state),
      },
    ),
  );
}

const _title = 'Restore original firmware';

/// The restore refused on a device that may not run it.
class _Unavailable extends StatelessWidget {
  const _Unavailable();

  @override
  Widget build(BuildContext context) => StepScaffold(
    leading: Leading.close,
    title: _title,
    content: [
      Notice(
        tone: Tone.fault,
        lead: 'This $deviceName cannot restore the original firmware.',
        text: 'Only the Android app can put it back. Restore from an Android phone that owns this feeder.',
      ),
    ],
  );
}

class _NoBackup extends StatelessWidget {
  const _NoBackup(this.flow, this.state);

  final RestoreFlow flow;
  final RestoreNoBackup state;

  @override
  Widget build(BuildContext context) => StepScaffold(
    appBarTitle: _title,
    title: 'No backup on this $deviceName',
    body:
        "The copy of the vendor firmware made during install isn't on this "
        '$deviceName. If you saved a copy when the install ended, pick that file.',
    content: [
      if (state.notStock != null)
        const Notice(
          tone: Tone.fault,
          lead: "That isn't the original firmware.",
          text:
              "It doesn't match the vendor's 1.1.15 exactly, so it can't go "
              'back on the feeder. It may be damaged or partial.',
        ),
    ],
    actions: [FilledButton(onPressed: flow.pickFile, child: const Text('Pick the file'))],
  );
}

class _Before extends StatelessWidget {
  const _Before(this.flow, this.state);

  final RestoreFlow flow;
  final RestoreBefore state;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final String name = flow.feederName;
    return StepScaffold(
      title: _title,
      body:
          'Puts back the vendor firmware that was on $name before Wellfed, '
          'from the backup made during install.',
      content: [
        FactList([
          ('Backup', Text(_backupWords(state))),
          ('Version', Text('Vendor ${state.version ?? ''}')),
          ('Checked', Text('Matches the original', style: TextStyle(color: c.ok))),
        ]),
        const SizedBox(height: Space.s4),
        const _Warning(
          "Meals and people set up in Wellfed don't carry over. Set the "
          "feeder up again in the vendor's app.",
        ),
        const SizedBox(height: Space.s3),
        _Warning(
          "After this, Wellfed can't talk to $name. If it doesn't come back "
          "on the vendor firmware, this app can't help from here.",
        ),
        const SizedBox(height: Space.s2),
        _Understood(state.understood, onChanged: flow.setUnderstood),
      ],
      actions: [
        FilledButton(
          style: FilledButton.styleFrom(backgroundColor: c.fault, foregroundColor: c.background),
          onPressed: state.understood ? flow.start : null,
          child: const Text(_title),
        ),
      ],
    );
  }

  static String _backupWords(RestoreBefore state) {
    if (state.picked) return 'From a picked file';
    final DateTime? at = state.backupDate;
    if (at == null) return 'On this $deviceName';
    const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
    return '${at.day} ${months[at.month - 1]}, on this $deviceName';
  }
}

class _Warning extends StatelessWidget {
  const _Warning(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        StrokeIcon(Glyph.warning, size: 18, color: c.warning),
        const SizedBox(width: 10),
        Expanded(
          child: Text(
            text,
            style: TextStyle(fontSize: Sizes.base, height: 1.5, color: c.text),
          ),
        ),
      ],
    );
  }
}

/// The agreement under the warnings: its box in their glyph column and its
/// words in their text column, the whole row one tap target.
class _Understood extends StatelessWidget {
  const _Understood(this.value, {required this.onChanged});

  final bool value;
  final ValueChanged<bool> onChanged;

  @override
  Widget build(BuildContext context) => MergeSemantics(
    child: InkWell(
      onTap: () => onChanged(!value),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 48),
        child: Row(
          children: [
            // The box is drawn 18 wide, as the warning glyph is; its own
            // tap area spills past that, inside the row's.
            SizedBox.square(
              dimension: 18,
              child: OverflowBox(
                maxWidth: 40,
                maxHeight: 40,
                child: Checkbox(
                  value: value,
                  materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
                  onChanged: (checked) => onChanged(checked ?? false),
                ),
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                'I understand',
                style: TextStyle(fontSize: Sizes.base, height: 1.5, color: context.colors.text),
              ),
            ),
          ],
        ),
      ),
    ),
  );
}

/// 12.2's steps, for the way back.
class _Running extends StatelessWidget {
  const _Running(this.flow, this.state);

  final RestoreFlow flow;
  final RestoreRunning state;

  @override
  Widget build(BuildContext context) => StepScaffold(
    appBarTitle: 'Restoring ${flow.feederName}',
    leading: state.committed ? Leading.none : Leading.back,
    content: [
      if (state.pressDispense) ...[
        const Notice(
          tone: Tone.warning,
          lead: 'Press Dispense on the feeder, once.',
          text: 'You have 60 seconds. The press shows you have the feeder in hand.',
        ),
        const SizedBox(height: Space.s4),
      ],
      Panel(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 14, 16, 2),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _Line('Press Dispense on the feeder', state.pressing),
              _Line('Sending', state.sending, fraction: state.written),
              _Line('Restart on the original firmware', state.restarting),
            ],
          ),
        ),
      ),
      const SizedBox(height: Space.s4),
      Text(
        state.sessionDropped
            ? 'The connection dropped. Trying to get it back. Keep the feeder '
                  'plugged in.'
            : 'Keep the feeder plugged in, and this $deviceName on the same Wi-Fi. '
                  "Once this has started, there's no stopping it from here.",
        style: bodyStyle(context),
      ),
      if (state.holdRefused) ...[
        const SizedBox(height: Space.s4),
        const Notice(tone: Tone.warning, text: 'Keep this screen open: Android would not let this run behind.'),
      ],
      WaitWords(
        state.pressDispense ? null : state.waitWords,
        progress: (state.pressing, state.sending, state.written, state.restarting),
      ),
    ],
  );
}

/// One line of the steps.
class _Line extends StatelessWidget {
  const _Line(this.text, this.state, {this.fraction});

  final String text;
  final CheckState state;

  /// How far a running step is, when it knows.
  final double? fraction;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final running = state == CheckState.running;
    final double? part = fraction;
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
              if (running && part != null)
                Text(
                  '${(part * 100).round()}%',
                  style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                ),
            ],
          ),
          if (running && part != null) ...[const SizedBox(height: Space.s2), Bar(value: part)],
        ],
      ),
    );
  }
}

class _Done extends StatelessWidget {
  const _Done(this.flow, this.state);

  final RestoreFlow flow;
  final RestoreDone state;

  @override
  Widget build(BuildContext context) => StepScaffold(
    leading: Leading.none,
    title: 'The original firmware is going back',
    body:
        '${flow.feederName} is restarting on vendor ${state.version ?? ''}. '
        "Wellfed can't talk to it any more. It still looks for this $deviceName, and "
        'keeps trying forever. Hold Reset for about 5 seconds, then set it up '
        "in the vendor's app.",
    actions: const [RestoreClose()],
  );
}

class RestoreSmall extends StatelessWidget {
  const RestoreSmall(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) => Text(
    text,
    style: TextStyle(fontSize: Sizes.sm, color: context.colors.textMuted),
  );
}

class RestoreClose extends StatelessWidget {
  const RestoreClose({super.key, this.outlined = false});

  final bool outlined;

  @override
  Widget build(BuildContext context) {
    void close() => Navigator.of(context).maybePop();
    return outlined
        ? OutlinedButton(onPressed: close, child: const Text('Done'))
        : FilledButton(onPressed: close, child: const Text('Done'));
  }
}
