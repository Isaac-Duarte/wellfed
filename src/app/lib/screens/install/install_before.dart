import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/install/install_screen.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

/// 2.1: what it takes.
class InstallBeforePage extends StatelessWidget {
  const InstallBeforePage({super.key, required this.flow, required this.state});

  final InstallFlow flow;
  final InstallBefore state;

  /// Why this phone cannot install: an APK with no image, or one whose
  /// sources cannot be reviewed. Refused here, before the feeder is touched.
  Widget? _refusal() {
    final (String lead, String text)? refusal = switch ((state.image, state.build)) {
      (ImageCheck_Absent(), _) => (
        'This build has no firmware to install.',
        'Install a release of the app, or build it with its image.',
      ),
      (ImageCheck_NotPinned(), _) => (
        'This build pins no firmware.',
        'It holds an image it has no pin for, so it will not install it.',
      ),
      (ImageCheck_Mismatch(:final String detail), _) => ("This build's firmware doesn't check out.", detail),
      (_, BuildCheck_Unreviewed(:final String commit)) => (
        'This build cannot send an install.',
        'Only a build of a clean commit can, so its sources can be '
            'reviewed. This one is $commit, with changes.',
      ),
      _ => null,
    };
    if (refusal == null) return null;
    final (String lead, String text) = refusal;
    return Notice(tone: Tone.fault, lead: lead, text: text);
  }

  Future<void> _begin(BuildContext context) async {
    final bool allowed = await flow.begin();
    if (allowed || !context.mounted) return;
    await showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Nearby devices is off'),
        content: const Text(
          'The install claims the feeder over Bluetooth. Allow "Nearby '
          'devices" for Wellfed in Android settings, then try again.',
        ),
        actions: [TextButton(onPressed: () => Navigator.pop(context), child: const Text('OK'))],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final Widget? refusal = _refusal();
    final bool damaged = state.pending == PendingInstall.damaged || state.pending == PendingInstall.ledgerDamaged;
    return StepScaffold(
      leading: Leading.close,
      appBarTitle: 'Install Wellfed',
      body:
          "Wellfed replaces the feeder's firmware. This phone keeps a backup "
          'of the original, so you can put it back later.',
      content: [
        const SectionLabel("You'll need"),
        const SizedBox(height: Space.s3),
        const InstallFact('The feeder plugged in, no food being dispensed'),
        const InstallFact('This phone on your home Wi-Fi, and its password'),
        const InstallFact('About 10 minutes with Wellfed open: this phone serves the firmware'),
        const SizedBox(height: Space.s4),
        const Notice(
          tone: Tone.warning,
          lead: "Its meals don't come across.",
          text:
              "Write down any set in the vendor's app, and set them up again "
              'in Wellfed afterwards.',
        ),
        if (refusal != null) ...[const SizedBox(height: Space.s4), refusal],
        if (state.bluetoothRefused) ...[const SizedBox(height: Space.s4), const InstallBluetoothRefused()],
        if (flow.mayStart && state.pending == PendingInstall.requestOpen) ...[
          const SizedBox(height: Space.s4),
          const Notice(
            tone: Tone.warning,
            lead: 'An earlier install has not finished.',
            text:
                'Its feeder may be running Wellfed, waiting for its owner, '
                'or still on its original firmware. The install is never sent '
                'to it a second time.',
          ),
        ],
        if (state.pending == PendingInstall.damaged) ...[
          const SizedBox(height: Space.s4),
          const Notice(
            tone: Tone.warning,
            lead: "This phone can't read its record of an earlier install.",
            text: 'Forget that install to go on. Its backups stay.',
          ),
        ],
        if (state.pending == PendingInstall.ledgerDamaged) ...[
          const SizedBox(height: Space.s4),
          const Notice(
            tone: Tone.warning,
            lead: "This phone can't read its record of an install request.",
            text: 'It treats that request as still open until you say otherwise.',
          ),
        ],
        if (state.repairFailed case final failed?) ...[
          const SizedBox(height: Space.s4),
          Notice(tone: Tone.fault, lead: "That didn't work.", text: failed),
        ],
      ],
      actions: [
        if (flow.mayStart && state.pending == PendingInstall.paired)
          OutlinedButton(onPressed: () => unawaited(flow.resume()), child: const Text('Pick up where it stopped')),
        if (flow.mayStart && state.pending == PendingInstall.requestOpen) ...[
          OutlinedButton(
            onPressed: () => unawaited(flow.listenForOpenRequest()),
            child: const Text('Listen for the feeder'),
          ),
          OutlinedButton(onPressed: () => unawaited(flow.resume()), child: const Text('Look for it over Bluetooth')),
        ],
        if (state.pending == PendingInstall.ledgerDamaged) InstallSetAsideLedger(flow: flow),
        // For a feeder past stock with no record to pick up from: Continue is
        // the Reset route, which is stock's.
        OutlinedButton(
          onPressed: flow.mayStart && !damaged ? flow.alreadyRuns : null,
          child: const Text('This feeder already runs Wellfed'),
        ),
        FilledButton(
          onPressed: flow.mayStart && !damaged ? () => unawaited(_begin(context)) : null,
          child: const Text('Continue'),
        ),
        if (flow.forgetOffered) InstallForgetInstallPage(flow: flow),
      ],
    );
  }
}

/// With no record of the install, only the owner can say which feeder this is: a
/// known feeder answering says nothing about it.
class InstallWhichFeederPage extends StatelessWidget {
  const InstallWhichFeederPage({super.key, required this.flow});

  final InstallFlow flow;

  @override
  Widget build(BuildContext context) => StepScaffold(
    onLeading: () => flow.back(InstallStage.before),
    appBarTitle: 'Install Wellfed',
    title: 'Which feeder?',
    body: 'This finishes the install on the feeder you choose.',
    content: [
      for (final feeder in flow.knownFeeders) ...[
        RouteCard(
          glyph: Glyph.install,
          title: feeder.name,
          body: 'Already on this phone.',
          onTap: () => unawaited(flow.pickFeeder(feeder.identity)),
        ),
        const SizedBox(height: Space.s3),
      ],
      RouteCard(
        glyph: Glyph.install,
        title: 'Another feeder, over Bluetooth',
        body: 'Hold it into setup mode with its buttons. This phone shows how.',
        onTap: () => unawaited(flow.pickFeeder(null)),
      ),
    ],
  );
}

/// Ask once before a repair that gives something up. `true` when the owner agreed.
Future<bool> _agreed(
  BuildContext context, {
  required String title,
  required String confirm,
  required List<String> words,
}) async {
  final bool? sure = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(title),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final (i, line) in words.indexed) ...[if (i > 0) const SizedBox(height: Space.s3), Text(line)],
          ],
        ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
        FilledButton(onPressed: () => Navigator.pop(context, true), child: Text(confirm)),
      ],
    ),
  );
  return sure == true;
}

/// The way out of a stuck or damaged install record, which keeps every backup and
/// never touches the stock ledger.
class InstallForgetInstallPage extends StatelessWidget {
  const InstallForgetInstallPage({super.key, required this.flow});

  final InstallFlow flow;

  Future<void> _confirm(BuildContext context) async {
    final bool sure = await _agreed(
      context,
      title: 'Forget this install?',
      confirm: 'Forget',
      words: [
        'This clears what this phone kept for the install in progress: which feeder it was installing on, and which feeder it trusted. It changes nothing on the feeder.',
        'Every backup of the original firmware stays on this phone.',
        if (flow.forgetEndsPickUp) '"Pick up where it stopped" ends here. To finish this feeder later, choose "This feeder already runs Wellfed" on the first screen.',
        if (flow.requestStillOpen) 'An install sent to a feeder has no known result. Wellfed stops listening for it, but keeps the record, so that feeder is never sent it a second time. If that feeder comes back, starting again waits until its result is known.',
      ],
    );
    if (sure) flow.forgetInstall();
  }

  @override
  Widget build(BuildContext context) =>
      TextButton(onPressed: () => unawaited(_confirm(context)), child: const Text('Forget this install'));
}

/// The owner's way past a stock ledger that does not parse. It gives up the guard
/// the ledger held, so the words say so, and nothing does this by itself.
class InstallSetAsideLedger extends StatelessWidget {
  const InstallSetAsideLedger({super.key, required this.flow});

  final InstallFlow flow;

  Future<void> _confirm(BuildContext context) async {
    final bool sure = await _agreed(
      context,
      title: 'Set the damaged record aside?',
      confirm: 'Set it aside',
      words: const [
        'This phone no longer knows whether an install it sent to a feeder is still open. If you go on, it could send that feeder the install again.',
        'Check that feeder first. Wait until it has finished anything it started, and see whether it runs Wellfed or its original firmware.',
        'The record is renamed and kept on this phone, not deleted. Every backup stays.',
      ],
    );
    if (sure) flow.setAsideLedgers();
  }

  @override
  Widget build(BuildContext context) =>
      OutlinedButton(onPressed: () => unawaited(_confirm(context)), child: const Text('Set the damaged record aside'));
}

/// The owner's way past a kept backup a run stopped on. The comparison that
/// stopped it is not loosened: the file goes only by this choice.
class InstallSetAsideBackup extends StatelessWidget {
  const InstallSetAsideBackup({super.key, required this.flow, required this.path, required this.problem});

  final InstallFlow flow;
  final String path;
  final BackupProblem problem;

  Future<void> _confirm(BuildContext context) async {
    final bool sure = await _agreed(
      context,
      title: 'Set this backup aside?',
      confirm: 'Set it aside',
      words: const [
        'That copy of the original firmware can no longer be used to go back to it.',
        "Wellfed reads a fresh copy only if the feeder still holds its original firmware. If it doesn't, this phone has no copy left to go back with.",
        'The file is renamed and kept on this phone, not deleted.',
      ],
    );
    if (sure) unawaited(flow.setAsideBackup(path, problem));
  }

  @override
  Widget build(BuildContext context) =>
      OutlinedButton(onPressed: () => unawaited(_confirm(context)), child: const Text('Set this backup aside'));
}

/// Android refused what a pick-up may need.
class InstallBluetoothRefused extends StatelessWidget {
  const InstallBluetoothRefused({super.key});

  @override
  Widget build(BuildContext context) => const Notice(
    tone: Tone.fault,
    lead: 'Nearby devices is off.',
    text:
        'The install claims the feeder over Bluetooth. Allow "Nearby devices" '
        'for Wellfed in Android settings, then try again.',
  );
}

/// 2.2: hold Reset. It erases the feeder's settings, so the owner says they
/// understand first.
class InstallResetPage extends StatefulWidget {
  const InstallResetPage({super.key, required this.flow, required this.state});

  final InstallFlow flow;
  final InstallReset state;

  @override
  State<InstallResetPage> createState() => _ResetState();
}

class _ResetState extends State<InstallResetPage> {
  bool _understood = false;

  @override
  Widget build(BuildContext context) {
    final InstallFlow flow = widget.flow;
    final offWifi = widget.state.phoneIp == null;
    return StepScaffold(
      stepLabel: 'Step 1 of 5',
      progress: 0.2,
      onLeading: () => flow.back(InstallStage.wifi),
      title: 'Reset the feeder',
      content: [
        const InstallFact('If the lock light is on, hold the lock button for 5 seconds.'),
        const InstallFact('Hold the rightmost button, Reset, for 5 seconds.'),
        const InstallFact('It restarts and opens its own Wi-Fi network for setup.'),
        const SizedBox(height: Space.s4),
        const Notice(
          tone: Tone.warning,
          lead: 'Resetting erases its meals and Wi-Fi.',
          text:
              "Everything saved by the vendor's app goes. Write the meals down "
              'first if you want them back.',
        ),
        const SizedBox(height: Space.s3),
        CheckboxListTile(
          contentPadding: EdgeInsets.zero,
          controlAffinity: ListTileControlAffinity.leading,
          value: _understood,
          onChanged: (value) => setState(() => _understood = value ?? false),
          title: const Text('I understand its meals will be erased'),
        ),
        if (offWifi) ...[
          const SizedBox(height: Space.s3),
          const Notice(
            tone: Tone.fault,
            lead: "This phone isn't on Wi-Fi.",
            text: 'Join your home Wi-Fi, then press the button again.',
          ),
        ],
      ],
      actions: [
        FilledButton(
          onPressed: _understood ? () => unawaited(flow.startHandover()) : null,
          child: const Text('I held Reset'),
        ),
      ],
    );
  }
}
