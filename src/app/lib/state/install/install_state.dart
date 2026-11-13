import 'package:flutter/foundation.dart';

import 'package:wellfed/src/rust/api/install.dart';

/// Which screen an install shows. The steps themselves are Rust's: this
/// follows what the run reports and carries the owner's answers back.
enum InstallStage {
  /// Reading the APK's image and this build's identity.
  loading,

  /// This device cannot install: it cannot join the feeder's own Wi-Fi.
  unavailable,

  /// What it takes, and how the feeder finds this phone.
  before,

  /// How feeders list this phone, when it has no name yet.
  naming,

  /// The Wi-Fi Bluetooth setup gives the replacement.
  wifi,

  /// The reset route: hold Reset, which erases the feeder's settings.
  reset,

  /// The reset route: join the feeder's network, send the Wi-Fi, come back.
  handover,

  /// A feeder connected: is it the owner's?
  confirm,

  /// With no record to go by: which feeder already runs Wellfed?
  which,

  /// Stock downloads the replacement from this phone.
  sending,

  /// This phone becomes the replacement's owner.
  claim,

  /// The backup, the promotion and the confirmation.
  finishing,

  /// It runs Wellfed, and the owner names it.
  done,

  /// It stopped, and the end says how.
  stopped,
}

/// How this device joins the feeder's setup network.
enum FeederJoin {
  /// Through Android's picker, which holds this process's sockets to it.
  picker,

  /// By the owner's hand, in a computer's own Wi-Fi menu, and back home the
  /// same way. The app only sees each switch happen.
  byHand,
}

/// Why a computer was not counted as on the feeder's network.
enum FeederJoinProblem {
  /// Its network never showed, or the owner stopped waiting.
  notSeen,

  /// It was already on another 192.168.1.x network, where the setup frame
  /// and the Wi-Fi password in it would go to that network's router.
  otherNetwork,

  /// Its address is one the feeder can't reach, as a VPN's is.
  vpn,

  /// It joined a new 192.168.1.x network, but the system wouldn't say which
  /// Wi-Fi that is, so the frame doesn't go.
  nameUnreadable,

  /// It was already on the feeder's network when the handover read where
  /// home is, so the frame would name the wrong address.
  alreadyOnFeeder,
}

/// Where the handover is.
enum Handover {
  /// The broker starts; nothing to press yet.
  starting,

  /// Waiting for the owner to join the feeder's network.
  join,

  /// Android's picker is up, or a computer waits for the owner to join.
  joining,

  /// The frame goes out, and this phone comes back.
  sending,

  /// The frame went out, and a computer waits for the owner to put it back
  /// on the home network, which the feeder must reach within stock's wait.
  returning,

  /// Waiting for the feeder to reach this phone.
  waiting,
}

/// Where the checklist is.
enum CheckState {
  /// Not reached yet.
  waiting,

  /// Going now.
  running,

  /// Finished.
  done,
}

/// What the listener after an uncertain request has heard.
enum Listening {
  /// Not started: the stop is not an uncertain one.
  idle,

  /// Listening now.
  running,

  /// Stock came back: it did not install. A new run may ask again only once
  /// the request is closed.
  stockStayed,

  /// Nothing, for the whole wait.
  silent,

  /// The listener could not run.
  failed,

  /// A stock ledger does not parse: setting it aside lets it listen.
  ledgerDamaged,

  /// Stock's server could not start: another app may hold its port.
  serverFailed,
}

/// What holds across an install's stages.
@immutable
final class InstallFacts {
  /// The facts at the start of an install.
  const InstallFacts({
    this.pending = PendingInstall.nothing,
    this.bluetoothRefused = false,
    this.bluetoothOff = false,
    this.repairFailed,
    this.phoneIp,
  });

  /// What an earlier run left on the feeder.
  final PendingInstall pending;

  /// Android refused Bluetooth to a pick-up that may need it.
  final bool bluetoothRefused;

  /// This phone's Bluetooth is off, and the run needs it to find the feeder.
  final bool bluetoothOff;

  /// Why a repair of this phone's records did not work.
  final String? repairFailed;

  /// Where the feeder reaches this phone.
  final String? phoneIp;
}

/// How one install stands: one class for each screen it can show.
sealed class InstallState {
  /// A state with these [facts].
  const InstallState(this.facts);

  /// What holds across the stages.
  final InstallFacts facts;

  /// What an earlier run left on the feeder.
  PendingInstall get pending => facts.pending;

  /// Android refused Bluetooth to a pick-up that may need it.
  bool get bluetoothRefused => facts.bluetoothRefused;

  /// This phone's Bluetooth is off, and the run needs it to find the feeder.
  bool get bluetoothOff => facts.bluetoothOff;

  /// Why a repair of this phone's records did not work.
  String? get repairFailed => facts.repairFailed;

  /// Where the feeder reaches this phone.
  String? get phoneIp => facts.phoneIp;
}

/// Reading the APK's image and this build's identity.
final class InstallLoading extends InstallState {
  /// Nothing known yet.
  const InstallLoading(super.facts);
}

/// This device cannot install, so nothing is read and the feeder is never
/// touched.
final class InstallUnavailable extends InstallState {
  /// Refused before anything else.
  const InstallUnavailable(super.facts);
}

/// What it takes, and how the feeder finds this phone.
final class InstallBefore extends InstallState {
  /// The first screen.
  const InstallBefore(super.facts, {required this.image, required this.build});

  /// The APK's image, once checked.
  final ImageCheck? image;

  /// This build's identity, once checked.
  final BuildCheck? build;
}

/// How feeders list this phone, when it has no name yet.
final class InstallNaming extends InstallState {
  /// The name step.
  const InstallNaming(super.facts);
}

/// The Wi-Fi Bluetooth setup gives the replacement.
final class InstallWifi extends InstallState {
  /// The Wi-Fi step.
  const InstallWifi(super.facts);
}

/// The reset route: hold Reset, which erases the feeder's settings.
final class InstallReset extends InstallState {
  /// The reset step.
  const InstallReset(super.facts);
}

/// The reset route: join the feeder's network, send the Wi-Fi, come back.
final class InstallHandover extends InstallState {
  /// The handover.
  const InstallHandover(
    super.facts, {
    required this.handover,
    required this.joinFailed,
    required this.restartAgain,
    required this.waitReason,
    this.join = FeederJoin.picker,
    this.homeNetwork,
    this.joinProblem,
  });

  /// Why a computer was not counted as on the feeder's network, with
  /// [joinFailed].
  final FeederJoinProblem? joinProblem;

  /// Where the handover is.
  final Handover handover;

  /// Android's picker ended with no network: the feeder's network is not up
  /// yet, or the owner cancelled. On a computer, the owner stopped waiting.
  final bool joinFailed;

  /// How this device joins the feeder's network.
  final FeederJoin join;

  /// The Wi-Fi this device was on before it joined the feeder's, when the
  /// system said: the one to go back to.
  final String? homeNetwork;

  /// The home network uses the feeder's setup addresses (192.168.1.x), so a
  /// computer can't reach the feeder while it stays on its home network too.
  bool get homeSharesSetupAddresses => phoneIp?.startsWith('192.168.1.') ?? false;

  /// Stock connected but its reports were lost: restart it once more.
  final bool restartAgain;

  /// Why stock has not moved on.
  final WaitReason? waitReason;
}

/// A feeder connected: is it the owner's?
final class InstallConfirm extends InstallState {
  /// A feeder to recognize.
  const InstallConfirm(super.facts, {required this.device, required this.deviceAddress});

  /// What the feeder says it is.
  final String? device;

  /// Where it connected from.
  final String? deviceAddress;
}

/// With no record to go by: which feeder already runs Wellfed?
final class InstallWhich extends InstallState {
  /// The feeder picker.
  const InstallWhich(super.facts);
}

/// Stock downloads the replacement from this phone.
final class InstallSending extends InstallState {
  /// The download.
  const InstallSending(super.facts, {required this.holdRefused, required this.percent, required this.restarting});

  /// Android refused the foreground service, so only staying on screen keeps
  /// the broker alive.
  final bool holdRefused;

  /// How far stock's download has gone.
  final int percent;

  /// Stock is restarting into Wellfed.
  final bool restarting;
}

/// This phone becomes the replacement's owner.
final class InstallClaim extends InstallState {
  /// The claim.
  const InstallClaim(
    super.facts, {
    required this.code,
    required this.deadline,
    required this.pressDispense,
    required this.searchingBluetooth,
    required this.settingUpAgain,
    required this.setupHint,
    required this.windowAsked,
  });

  /// The code as panel positions; empty until the feeder asks for one.
  final List<int> code;

  /// When the feeder stops taking presses.
  final DateTime? deadline;

  /// Waiting on the owner to press Dispense.
  final bool pressDispense;

  /// The run looks for a feeder advertising Bluetooth setup, rather than for
  /// one already on the owner's Wi-Fi.
  final bool searchingBluetooth;

  /// The owner gave a new Wi-Fi for a feeder that never joined: the run goes
  /// through Bluetooth setup again, and the claim screen says how to get the
  /// feeder there.
  final bool settingUpAgain;

  /// Bluetooth has found nothing for a while: a feeder that is not advertising
  /// by itself takes the owner's hold.
  final bool setupHint;

  /// A bench image's recovery window is waiting for the owner.
  final bool windowAsked;
}

/// The backup, the promotion and the confirmation.
final class InstallFinishing extends InstallState {
  /// The checklist.
  const InstallFinishing(
    super.facts, {
    required this.joining,
    required this.copying,
    required this.copied,
    required this.checked,
    required this.promoting,
    required this.written,
    required this.confirming,
  });

  /// The feeder restarting onto the owner's Wi-Fi, after Bluetooth setup.
  final CheckState joining;

  /// Copying the original firmware.
  final CheckState copying;

  /// How much of it came across, from 0 to 1.
  final double copied;

  /// Checking the copy.
  final CheckState checked;

  /// Making Wellfed permanent.
  final CheckState promoting;

  /// How much of it went across, from 0 to 1.
  final double written;

  /// Restarting and confirming.
  final CheckState confirming;
}

/// It runs Wellfed, and the owner names it.
final class InstallDone extends InstallState {
  /// A finished install.
  const InstallDone(super.facts, {required this.backupPath});

  /// A checked copy of stock.
  final String? backupPath;
}

/// It stopped, and [stop] says how.
final class InstallStopped extends InstallState {
  /// A stopped install.
  const InstallStopped(
    super.facts, {
    required this.stop,
    required this.listening,
    required this.listened,
    required this.listeningForEarlier,
    required this.stockSeen,
    required this.setupSeen,
    required this.listenerDetail,
    this.promotionMayHaveGone = false,
  });

  /// How it ended.
  final InstallEnd_Stopped stop;

  /// An earlier run of this install stopped unsure whether its promotion
  /// went in, so the feeder may no longer hold its original firmware.
  final bool promotionMayHaveGone;

  /// What the listener after an uncertain request has heard.
  final Listening listening;

  /// How long it has listened.
  final Duration listened;

  /// This listen is for a request an earlier run left, not one this run sent.
  final bool listeningForEarlier;

  /// Stock has been heard since.
  final bool stockSeen;

  /// A feeder nearby advertises setup while this listens. It says nothing about
  /// which feeder, so only the owner can say it is this one.
  final bool setupSeen;

  /// What the listener said, if it could not run.
  final String? listenerDetail;
}
