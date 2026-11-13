import 'dart:async';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/install/install_before.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';

/// 2.9: why it stopped, what the feeder is left with, and what next. An
/// uncertain install is listened for and never sent again.
class InstallStoppedPage extends StatelessWidget {
  const InstallStoppedPage({super.key, required this.flow, required this.state});

  final InstallFlow flow;
  final InstallStopped state;

  /// The design's other ways an install stops, for the owner who meets one
  /// later. The stop on the screen, `shown`, is not listed again.
  List<(String, String)> _others(StopCase shown) => [
    if (shown is! StopCase_OtherFirmware)
      ('Different vendor firmware', 'Wellfed installs over 1.1.15 only, for now. Nothing on the feeder changed.'),
    if (shown is! StopCase_TwoFeeders) _words(const StopCase.twoFeeders()),
    if (shown is! StopCase_BackupMismatch) _words(const StopCase.backupMismatch()),
  ];

  /// What the feeder holds beside Wellfed, as far as this install knows: a
  /// promotion that may have gone in may have replaced its original firmware.
  String _stockHeld() => state.promotionMayHaveGone
      ? 'the backup of its original firmware stays on this $deviceName'
      : 'still holds its original firmware';

  (String, String) _words(StopCase stop) => switch (stop) {
    StopCase_Uncertain() => switch (state.listening) {
      Listening.stockStayed when state.stop.resume == Resume.fromStart => (
        "It didn't install",
        'The feeder came back on its original firmware, and nothing changed '
            'on it. You can start again.',
      ),
      Listening.stockStayed => (
        'A new install has to wait',
        'The feeder is on its original firmware, but it has not stayed connected '
            "long enough to rule out the earlier install. A new one isn't "
            'possible until it has stayed connected to this $deviceName for 10 minutes '
            'without a break. Leave it plugged in, keep Wellfed open and listen '
            'again.',
      ),
      Listening.silent => (
        'Still no word from the feeder',
        'It may have installed, or it may be off the network. Nothing was '
            'sent a second time. If it shows no Wi-Fi light, look for it '
            'over Bluetooth.',
      ),
      Listening.failed => (
        "We can't listen for the feeder",
        state.listenerDetail ?? 'Something on this $deviceName stopped it.',
      ),
      Listening.ledgerDamaged => (
        "This $deviceName can't read an install request's record",
        'Nothing was sent a second time. Until you set the record aside, '
            'Wellfed treats that request as still open and cannot listen for it.',
      ),
      Listening.serverFailed => (
        "We can't listen for the feeder",
        'This $deviceName could not start what listens for it. Another app may '
            'hold the port it needs. Nothing was sent a second time.',
      ),
      _ when state.listeningForEarlier => (
        'Waiting to hear from the feeder',
        'This $deviceName never heard how an earlier install ended, even if you have '
            'put the original firmware back since. Leave the feeder plugged in. '
            'That install closes 20 minutes after it went out, once the feeder '
            'has reported its version and been quiet for 10. Nothing is sent a '
            'second time.',
      ),
      _ => (
        "We don't know yet if it installed",
        'The feeder stopped answering partway through. That can be normal '
            'while it restarts. Leave it plugged in. Wellfed listens for up '
            'to 30 minutes and never sends the install a second time.',
      ),
    },
    StopCase_UncertainWrite(:final write) => (
      "We don't know if that last step ran",
      switch (write) {
        UncertainWrite.promotion =>
          'The feeder stopped answering while Wellfed became permanent. Leave '
              'it plugged in: picking up reads where it got to.',
      },
    ),
    StopCase_OtherFirmware(:final version) => (
      'Different vendor firmware',
      'This feeder runs $version. Wellfed installs over 1.1.15 only, for now. '
          'Nothing on the feeder changed.',
    ),
    StopCase_TwoFeeders() => (
      'Two feeders answered',
      "We can't tell which one is yours. Unplug the others and start again.",
    ),
    StopCase_BackupMismatch() => (
      "The backup didn't match",
      "Wellfed won't make itself permanent without a good backup, so this "
          'stop changed nothing on the feeder.',
    ),
    StopCase_NotYourFeeder() => (
      "That wasn't your feeder",
      'Nothing was sent to it. Check what else on your network could use that '
          'name, then start again.',
    ),
    StopCase_NoFeederCame() => (
      'No feeder connected',
      'Nothing was sent. Check that it is plugged in, then start again.',
    ),
    StopCase_HandoverFailed() => (
      "This $deviceName couldn't finish the handover",
      "Nothing was installed. Check that the feeder's setup network is up and "
          'that this $deviceName is on your Wi-Fi, then start again. If its setup '
          'network is gone, hold Reset again first.',
    ),
    StopCase_NotConfirmed() => (
      "Wellfed hasn't confirmed yet",
      'It is installed and may still be checking itself. Stay near it and pick '
          'up where it stopped to look again.',
    ),
    StopCase_FrameNotTaken() => (
      "The feeder didn't reach this $deviceName",
      'It should keep nothing and open its setup network again. '
          'Check the Wi-Fi password, that your Wi-Fi has a 2.4 GHz band, and '
          'that this $deviceName stayed on it.',
    ),
    StopCase_EarlierRequestOpen() => (
      'An earlier install is still open',
      'This $deviceName never heard how it ended, even if you have put the original '
          'firmware back since, so Wellfed will not ask again yet. Listen with '
          'the feeder plugged in: 20 minutes after that install, once it has '
          'reported its version and been quiet for 10, you can start again.',
    ),
    StopCase_RequestRefused(:final why) => switch (why) {
      RequestRefusal.stale => (
        "The feeder's reports got too old",
        'Wellfed only sends the install right after the feeder reports in. '
            'Nothing was sent. Restart the feeder and start again.',
      ),
      RequestRefusal.offline => (
        'The feeder went offline',
        'It left this $deviceName before the install went out, and nothing was '
            'sent. Check that it is plugged in, then start again.',
      ),
      RequestRefusal.imageServer => (
        "This $deviceName wasn't ready to serve the firmware",
        'Nothing was sent. Start again.',
      ),
      RequestRefusal.other => (
        "The install wasn't sent",
        'Nothing reached the feeder. See below for what this $deviceName said.',
      ),
    },
    StopCase_StockRefused(:final code) => (
      'The feeder refused the install',
      code == null
          ? 'It keeps running its original firmware.'
          : 'It keeps running its original firmware (error $code).',
    ),
    StopCase_ClockUnset() => (
      "The feeder's clock isn't set",
      'It refused before downloading, and nothing was sent. Restart the '
          'feeder and start again.',
    ),
    StopCase_NoSetupFeeder() => (
      'No feeder is waiting over Bluetooth',
      'It may still be starting. Keep this $deviceName near it and try again.',
    ),
    StopCase_SetupFailed(:final refusal) => (
      "This $deviceName couldn't claim it",
      switch (refusal) {
        SetupRefusal.wrongCode =>
          'The feeder heard a different order of presses, or another device '
              'is in the way. Try again with a new code.',
        SetupRefusal.unconfirmed =>
          'The code works for 60 seconds, and the time ran out. Try again with '
              'a new code.',
        SetupRefusal.busy => 'Another phone is setting it up. Wait, then try again.',
        SetupRefusal.full => 'It holds 8 phones and servers. An owner removes one first.',
        SetupRefusal.notWritable => "It can't save right now. Try again in a minute.",
        SetupRefusal.invalid => 'It rejected the $deviceName name or the Wi-Fi. Check them and start again.',
        null =>
          'The Bluetooth exchange broke off. Move closer and try again. It '
              'runs Wellfed and ${_stockHeld()}.',
      },
    ),
    StopCase_OtherFeeder() => (
      'A different feeder answered',
      'The code was keyed in on a feeder this install does not know. It may be '
          'another feeder, or this one after a factory reset, which gives it a '
          'new identity. It accepted this $deviceName and may now hold your Wi-Fi '
          'password, and it stays known to this $deviceName even if you forget this '
          "install. To carry on with this install's feeder, put it in setup "
          'mode, then pick up and key in the code on its buttons. Otherwise, '
          'forget this install.',
    ),
    StopCase_LedgerDamaged() => (
      "This $deviceName can't read an install request's record",
      'Nothing was sent to the feeder. Until you say otherwise, Wellfed treats '
          'that request as still open.',
    ),
    StopCase_BackupDamaged() => (
      "This $deviceName's backup is damaged",
      'The copy of the original firmware it kept for this feeder no longer '
          'opens. Wellfed stays where it is on the feeder, and nothing was '
          'overwritten.',
    ),
    StopCase_BackupDiffers() => (
      "The backup doesn't match the one kept",
      'What the feeder holds differs from the copy this $deviceName kept, and a kept '
          "copy is never replaced. Wellfed won't make itself permanent until "
          'that is settled.',
    ),
    StopCase_DamagedRecord() => (
      "This $deviceName can't read its install record",
      'Nothing was sent to the feeder. Forget this install to go on. Every '
          'backup of the original firmware stays on this $deviceName.',
    ),
    StopCase_NotAnswering() => (
      'The feeder stopped answering',
      'It paired with this $deviceName, but does not answer on this network. Check '
          'that this $deviceName is on the network you gave it.',
    ),
    StopCase_NotJoined() => (
      "It didn't join your Wi-Fi",
      'It runs Wellfed and this $deviceName is its owner, but it never appeared on '
          'your network. The Wi-Fi password may be wrong, or this $deviceName may be '
          'on another network. Give it the Wi-Fi again: you put the feeder back '
          'in setup mode with its buttons, and this $deviceName shows how.',
    ),
    StopCase_PhoneOffWifi() => (
      "This $deviceName isn't on Wi-Fi",
      'Nothing was sent. Join your home network, then try again.',
    ),
    StopCase_WifiRefused() => (
      "That Wi-Fi can't be given to a feeder",
      'Nothing was sent. Check its name and password, and enter them again.',
    ),
    StopCase_NoWifiGiven() => (
      'This install needs your Wi-Fi',
      'Nothing was sent. The feeder has no other way to join your network. '
          'Enter its name and password.',
    ),
    StopCase_PromotionFailed() => (
      "Wellfed didn't become permanent",
      'The feeder runs Wellfed for now and ${_stockHeld()}.',
    ),
    StopCase_FellBack() => (
      'It went back after restarting',
      "The new copy didn't confirm, so the feeder returned to the one it was "
          'running.',
    ),
    StopCase_ServerFailed() => (
      "This $deviceName couldn't serve the feeder",
      'Another app may hold the port it needs. Nothing was sent.',
    ),
    StopCase_ImageRefused(:final check) =>
      imageRefusal(check) ?? ("This build's firmware doesn't check out.", 'Nothing was sent.'),
    StopCase_Cancelled() => ('You stopped the install', 'Nothing is waiting on it.'),
    StopCase_Other() => ('The install stopped', 'See below for what it said.'),
  };

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final InstallEnd_Stopped stop = state.stop;
    final uncertain = stop.case_ is StopCase_Uncertain;
    final (String title, String body) = _words(stop.case_);
    final bool listening = uncertain && state.listening == Listening.running;
    final int minutes = state.listened.inMinutes;
    final List<(String, String)> others = _others(stop.case_);

    return StepScaffold(
      leading: Leading.close,
      onLeading: () => context.go('/'),
      top: Align(
        alignment: Alignment.centerLeft,
        child: StrokeIcon(
          uncertain ? Glyph.clock : Glyph.failed,
          size: 40,
          color: uncertain ? c.warning : c.fault,
          weight: 2,
        ),
      ),
      title: title,
      body: body,
      content: [
        if (listening) ...[
          Searching(
            'Listening for the feeder · $minutes of '
            '${flow.listenWait.inMinutes} min',
          ),
          const SizedBox(height: Space.s2),
          Bar(value: flow.listenWait.inSeconds == 0 ? 0 : state.listened.inSeconds / flow.listenWait.inSeconds),
          if (state.stockSeen) ...[
            const SizedBox(height: Space.s3),
            const Notice(
              tone: Tone.neutral,
              lead: 'The feeder came back on its original firmware.',
              text: 'Wellfed is making sure before it asks again.',
            ),
          ],
          if (state.setupSeen) ...[
            const SizedBox(height: Space.s3),
            const Notice(
              tone: Tone.neutral,
              lead: 'A feeder nearby is waiting for setup.',
              text:
                  "If it's this one, it installed. If it's another feeder, "
                  'keep listening.',
            ),
            const SizedBox(height: Space.s2),
            OutlinedButton(onPressed: () => unawaited(flow.resume()), child: const Text('Set up this feeder')),
          ],
          const SizedBox(height: Space.s5),
        ],
        if (state.bluetoothRefused) ...[const InstallBluetoothRefused(), const SizedBox(height: Space.s4)],
        if (state.bluetoothOff) ...[InstallBluetoothOff(flow: flow), const SizedBox(height: Space.s4)],
        ..._nextSteps(context, stop),
        if (flow.mayForget &&
            state.pending == PendingInstall.ledgerDamaged &&
            stop.case_ is! StopCase_LedgerDamaged) ...[
          const SizedBox(height: Space.s2),
          InstallSetAsideLedger(flow: flow),
        ],
        if (flow.forgetOffered) ...[const SizedBox(height: Space.s2), InstallForgetInstallPage(flow: flow)],
        if (state.repairFailed case final failed?) ...[
          const SizedBox(height: Space.s2),
          Notice(tone: Tone.fault, lead: "That didn't work.", text: failed),
        ],
        for (final String detail in [
          if (stop.detail.isNotEmpty && !listening) stop.detail,
          // A listener's words, where the body says what to do instead.
          if (state.listening case Listening.ledgerDamaged || Listening.serverFailed)
            if (state.listenerDetail case final String detail?) detail,
        ]) ...[
          const SizedBox(height: Space.s4),
          Text(
            detail,
            style: TextStyle(fontFamily: Fonts.mono, fontSize: Sizes.xs, height: 1.4, color: c.textMuted),
          ),
        ],
        const SizedBox(height: Space.s6),
        const SectionLabel('Other ways an install stops'),
        const SizedBox(height: Space.s3),
        ReasonList(others),
      ],
    );
  }

  /// What the owner can do from here. Nothing here sends a write the run
  /// marked uncertain: a pick-up reads where the feeder got to, and a new
  /// request goes only after the ledger closed the old one.
  List<Widget> _nextSteps(BuildContext context, InstallEnd_Stopped stop) {
    return switch (stop.case_) {
      StopCase_Uncertain() => switch (state.listening) {
        Listening.running => const [],
        Listening.stockStayed when stop.resume == Resume.fromStart => [
          _Primary('Start again', () => unawaited(flow.again())),
        ],
        Listening.stockStayed => [
          _Primary('Listen again', () => unawaited(flow.listen())),
          const SizedBox(height: Space.s2),
          const _Home(),
        ],
        _ => [
          _Primary('Listen again', () => unawaited(flow.listen())),
          const SizedBox(height: Space.s2),
          _Secondary('Look for it over Bluetooth', () => unawaited(flow.resume())),
          const _Home(),
        ],
      },
      StopCase_LedgerDamaged() => [InstallSetAsideLedger(flow: flow), const _Home()],
      StopCase_BackupDamaged(:final String path) => [
        InstallSetAsideBackup(flow: flow, path: path, problem: BackupProblem.damaged),
        const SizedBox(height: Space.s2),
        const _Home(),
      ],
      StopCase_BackupDiffers(:final String path) => [
        InstallSetAsideBackup(flow: flow, path: path, problem: BackupProblem.differs),
        const SizedBox(height: Space.s2),
        const _Home(),
      ],
      StopCase_EarlierRequestOpen() => [_Primary('Listen for it', () => unawaited(flow.listenForOpenRequest()))],
      StopCase_ClockUnset() => [_Primary('Start again', () => unawaited(flow.again()))],
      StopCase_TwoFeeders() ||
      StopCase_NotYourFeeder() => [_Primary('Start again', () => unawaited(flow.again(forget: true)))],
      StopCase_NotJoined() => [
        _Primary('Enter the Wi-Fi again', flow.newWifiForSetup),
        const SizedBox(height: Space.s2),
        _Secondary('Keep waiting for it', () => unawaited(flow.keepWaiting())),
      ],
      StopCase_FrameNotTaken() => [
        _Primary('Enter the Wi-Fi again', flow.changeWifi),
        _Secondary('Try again', () => unawaited(flow.again())),
      ],
      StopCase_WifiRefused() || StopCase_NoWifiGiven() => [_Primary('Enter the Wi-Fi again', flow.changeWifi)],
      _ => switch (stop.resume) {
        Resume.fromStart => [_Primary('Start again', () => unawaited(flow.again()))],
        Resume.fromBank0 => [_Primary('Pick up where it stopped', () => unawaited(flow.again()))],
        Resume.nothing => [const _Home()],
      },
    };
  }
}

class _Primary extends StatelessWidget {
  const _Primary(this.label, this.onPressed);

  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => FilledButton(onPressed: onPressed, child: Text(label));
}

class _Secondary extends StatelessWidget {
  const _Secondary(this.label, this.onPressed);

  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => OutlinedButton(onPressed: onPressed, child: Text(label));
}

class _Home extends StatelessWidget {
  const _Home();

  @override
  Widget build(BuildContext context) => TextButton(onPressed: () => context.go('/'), child: const Text('Go to Home'));
}
