import 'package:flutter/material.dart';

import 'package:wellfed/src/limits.g.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/widgets/desktop.dart';

/// Why a request came to nothing, or may have: `null` once it worked. An
/// uncertain write says so, and nothing sends it again.
String? replyProblem(FeederReply reply) => switch (reply) {
  FeederReply_Refused(:final refusal) => refusalWords(refusal),
  FeederReply_NotAsked(:final reason) => notAskedWords(reason),
  FeederReply_Unanswered() =>
    "No answer came, so it may or may not have gone through. Wellfed won't "
        'send it again.',
  FeederReply_NotSent(:final reason, :final server) => notSentWords(reason, server?.kind),
  FeederReply_Unexpected() => 'The feeder answered something unexpected.',
  FeederReply_Unreadable() =>
    "The feeder runs newer firmware and answered in a way this app can't read, so "
        "it may or may not have gone through. Wellfed won't send it again. Update this app.",
  _ => null,
};

/// Why a request never reached the feeder. Nothing changed, whatever the
/// cause, and the cause says what to do next.
String notSentWords(NotSentReason reason, ServerErrorKind? server) => switch ((reason, server)) {
  (_, ServerErrorKind.tokenRevoked) => 'The server no longer takes this $deviceName. Log in again. Nothing changed.',
  (_, ServerErrorKind.accountDisabled) =>
    'This account on the server has expired, so it sent nothing. An owner can extend it.',
  (_, ServerErrorKind.forbidden) => 'Your account on the server may not do that. Nothing changed.',
  (_, ServerErrorKind.feederOffline) => "The server can't reach the feeder right now. Nothing changed.",
  (_, ServerErrorKind.unreachable) => "The server didn't answer, so this didn't go. Nothing changed.",
  (NotSentReason.server, _) => "The server didn't pass this on. Nothing changed.",
  (NotSentReason.noPath, _) => "This $deviceName can't reach the feeder right now. Nothing changed.",
  (NotSentReason.store || NotSentReason.random, _) =>
    "This $deviceName couldn't note it down before sending, so it didn't send it. Nothing changed.",
  (NotSentReason.setAside, _) =>
    'A record of changes that may have gone was set aside, so this waits until you say the bowls are checked.',
  (NotSentReason.session, _) => 'The connection dropped before this went out. Nothing changed.',
};

String refusalWords(FeederRefusal refusal) => switch (refusal) {
  FeederRefusal_Forbidden() => "This $deviceName isn't allowed to do that here.",
  FeederRefusal_Stale() => 'This changed just now. Look again and decide again.',
  FeederRefusal_Busy() => "It's busy. Try again in a moment.",
  FeederRefusal_Unavailable() => "This feeder's firmware doesn't do that yet.",
  FeederRefusal_Invalid() => 'The feeder refused that value.',
  FeederRefusal_NotWritable() => "It can't save changes right now.",
  FeederRefusal_Full() => 'Every slot is taken.',
  FeederRefusal_NotFound() => "It doesn't have that any more.",
  FeederRefusal_Muted() => 'Sound is off, or outside its hours.',
  FeederRefusal_Time() => 'The feeder disagreed with this clock.',
  FeederRefusal_Fault() => 'It stopped on a fault and needs a restart.',
  FeederRefusal_Update() => 'The feeder refused the update.',
  FeederRefusal_Unverified() => 'This feeder has not verified that mode.',
  FeederRefusal_Portions(:final asked?) =>
    'A feed is ${Limits.portionsLeast} to ${Limits.portionsMost} portions, not $asked.',
  FeederRefusal_Portions() => 'A feed is ${Limits.portionsLeast} to ${Limits.portionsMost} portions.',
  FeederRefusal_NoClock() => "The feeder doesn't know the date. Set its time first.",
  FeederRefusal_Unknown() => 'The feeder refused it for a reason this app does not know. Update this app.',
};

String notAskedWords(NotAsked reason) => switch (reason) {
  NotAsked.credentials =>
    'A network name is 1 to 32 bytes, and its password 8 to 63 plain '
        'characters.',
  NotAsked.emptySlot => "That meal isn't there any more.",
  NotAsked.plansFull => 'A feeder keeps ${Limits.meals} meals, and this one has ${Limits.meals}.',
  NotAsked.nothingToChange => 'Nothing changed.',
  NotAsked.badPlan =>
    'A meal is ${Limits.portionsLeast} to ${Limits.portionsMost} portions, with up to ${Limits.callPlaysMost} calls.',
  NotAsked.badWeight => 'Give the weight in grams, like 9.8.',
  NotAsked.badNtp => 'Give an IPv4 address, like 192.168.1.1.',
  NotAsked.badZone => "The feeder can't follow that time zone.",
  NotAsked.badRecording => 'Use a WAV recording of about 8 seconds at most.',
  NotAsked.badKey => "That isn't a client's key.",
  NotAsked.held => "It waited behind a change that wasn't confirmed, so it wasn't sent.",
};

/// Say why `reply` came to nothing, if it did; whether it worked.
bool showProblem(BuildContext context, FeederReply reply) {
  final String? problem = replyProblem(reply);
  if (problem == null) return true;
  ScaffoldMessenger.of(context)
    ..hideCurrentSnackBar()
    ..showSnackBar(SnackBar(content: Text(problem)));
  return false;
}
