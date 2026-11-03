import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/restore/restore_screen.dart';
import 'package:wellfed/src/rust/api/restore.dart';
import 'package:wellfed/state/restore/restore.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/layout.dart';

class RestoreUncertainPage extends StatefulWidget {
  const RestoreUncertainPage(this.flow, this.state, {super.key});

  final RestoreFlow flow;
  final RestoreUncertain state;

  @override
  State<RestoreUncertainPage> createState() => _UncertainState();
}

class _UncertainState extends State<RestoreUncertainPage> {
  RestoreFlow get flow => widget.flow;
  RestoreUncertain get state => widget.state;

  @override
  void initState() {
    super.initState();
    flow.startLook();
  }

  @override
  void dispose() {
    flow.stopLook();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final String name = flow.feederName;
    final (String title, String body) = switch (state.look) {
      RestoreLook.looking => ("We can't tell if it went through", _why()),
      RestoreLook.wellfed => (
        '$name is still running Wellfed',
        "The restore didn't finish, and nothing is lost. You can restore "
            'again; you press Dispense on the feeder once more.',
      ),
      RestoreLook.sighted => (
        "We still can't tell",
        '$name showed up on your Wi-Fi, but it never answered this $deviceName as '
            'idle. It may be finishing or restarting. Close this and check '
            "again in a few minutes, here or in the vendor's app.",
      ),
      RestoreLook.setup => (
        'A feeder nearby is waiting to be set up',
        "If that's $name, the restore didn't finish, and Wellfed had already "
            'cleared its Wi-Fi, its phones and its meals to make way. Your '
            'backup is still good: set it up, then restore from there if you '
            'still want to. If you have another feeder in setup, this may be '
            'that one, and $name may be on the original firmware: check in the '
            "vendor's app.",
      ),
      RestoreLook.nothing => (
        "It's probably on the original firmware",
        "$name didn't show up on your Wi-Fi or over Bluetooth. Check in the "
            "vendor's app: if it shows up there, the restore worked.",
      ),
    };
    return StepScaffold(
      leading: Leading.none,
      title: title,
      body: body,
      content: [
        switch (state.look) {
          RestoreLook.looking => const Searching('Looking for it on your Wi-Fi and over Bluetooth…'),
          RestoreLook.wellfed || RestoreLook.sighted => const SizedBox.shrink(),
          RestoreLook.setup => const Notice(
            tone: Tone.warning,
            lead: "Don't hold Reset yet.",
            text:
                'If this feeder is on Wellfed, that erases the identity your '
                "backup is filed under. Check the vendor's app first.",
          ),
          RestoreLook.nothing => const Notice(
            tone: Tone.warning,
            lead: "If the vendor's app can't find it,",
            text:
                'hold Reset for about 5 seconds and set it up there. Only do '
                'this once you are sure it is not on Wellfed: on Wellfed, that '
                'hold erases the identity your backup is filed under.',
          ),
        },
        if (state.detail case final detail?) ...[const SizedBox(height: Space.s3), RestoreSmall(detail)],
      ],
      actions: [
        if (state.look == RestoreLook.wellfed) FilledButton(onPressed: flow.again, child: const Text('Restore again')),
        if (state.look == RestoreLook.setup)
          FilledButton(onPressed: () => context.push('/setup'), child: const Text('Set it up')),
        RestoreClose(outlined: state.look == RestoreLook.wellfed || state.look == RestoreLook.setup),
      ],
    );
  }

  String _why() => switch (state.why) {
    UncertainWhy.stopped =>
      'This was stopped as the firmware was starting to go across, or after it '
          'had. Nothing more is sent.',
    UncertainWhy.failed =>
      'This $deviceName hit a problem as the firmware was starting to go across, or '
          'after it had. Nothing more is sent.',
    _ =>
      '${flow.feederName} stopped answering after the firmware started going '
          'across. Nothing more is sent. Keep it plugged in while this looks.',
  };
}

class RestoreStoppedPage extends StatelessWidget {
  const RestoreStoppedPage(this.flow, this.state, {super.key});

  final RestoreFlow flow;
  final RestoreStopped state;

  @override
  Widget build(BuildContext context) {
    final (String title, String body) = _words(state.stop, flow.feederName);
    return StepScaffold(
      leading: Leading.none,
      title: title,
      body: body,
      content: [if (state.detail case final detail?) RestoreSmall(detail)],
      actions: [
        if (state.mayRetry) FilledButton(onPressed: flow.again, child: const Text('Try again')),
        RestoreClose(outlined: state.mayRetry),
      ],
    );
  }

  static (String, String) _words(RestoreStop? stop, String name) => switch (stop) {
    RestoreStop.noPress => ('Dispense was not pressed in time', '$name keeps running Wellfed, and nothing was sent.'),
    RestoreStop.enrollmentUnknown => (
      "We don't know if the press got through",
      '$name stopped answering just then. It keeps running Wellfed, and '
          'trying again finds out first, without asking for a second '
          'press if it took.',
    ),
    RestoreStop.keyRefused => (
      "$name wouldn't take the press",
      'It may be busy with another request, or its list of phones and '
          'keys may be full. It keeps running Wellfed, and nothing was '
          'sent.',
    ),
    RestoreStop.notOwner => (
      'Only the owner can do this',
      "This $deviceName isn't the owner of $name, which keeps running Wellfed.",
    ),
    RestoreStop.notAnswering => (
      "$name isn't answering",
      'Nothing was sent. Check that it is plugged in and this $deviceName is '
          'on the same Wi-Fi.',
    ),
    RestoreStop.noUpdates => (
      "$name can't take this",
      'The firmware it runs takes no updates from this app. Nothing was '
          'sent.',
    ),
    RestoreStop.backupRefused => (
      "That isn't the original firmware",
      "It doesn't match the vendor's 1.1.15 exactly, so nothing was sent.",
    ),
    RestoreStop.keyFailed => (
      "This $deviceName couldn't set up the restore",
      '$name keeps running Wellfed, and nothing was sent. The backup is '
          'fine: this is about space or storage on this $deviceName.',
    ),
    RestoreStop.installRunning => (
      'An install is running on $name',
      '$name is busy with another install, so this restore sent nothing. '
          'Wait for it to finish, then try again.',
    ),
    RestoreStop.cancelUnknown => (
      "$name didn't clear an earlier install",
      'An earlier install was cut short, and it has to go before the original '
          'firmware can. $name keeps running Wellfed, and nothing was sent. '
          'Try again in a minute.',
    ),
    RestoreStop.keyGone => (
      'The restore lost its key',
      '$name keeps running Wellfed, and nothing was sent. An earlier '
          'restore on this $deviceName cleared the key as this one was about to '
          'use it. Try again; you may be asked to press Dispense once more.',
    ),
    RestoreStop.refused => (
      '$name said no',
      "It keeps running Wellfed, though the vendor's copy may be partly "
          "written. This isn't sent again from here. If you want to try again, "
          'unplug the feeder for a few seconds first, since it can refuse '
          'until its power has been off, then start a new restore.',
    ),
    RestoreStop.cancelled => ('Stopped', '$name keeps running Wellfed, and nothing was sent.'),
    RestoreStop.other || null => ('The restore stopped', '$name most likely keeps running Wellfed.'),
  };
}
