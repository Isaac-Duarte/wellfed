import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/update.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/update/update.dart';
import 'package:wellfed/state/update/update_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/replies.dart';

/// What an update's end comes to: a title and what it means (12.3).
(String, String) updateEndWords(String name, UpdateEnd end) => switch (end) {
  UpdateEnd_Confirmed() => ('Updated', '$name runs the new version and has kept it.'),
  UpdateEnd_AlreadyRunning() => ('Nothing to do', '$name already runs this version. Nothing was sent.'),
  UpdateEnd_Refused(:final refusal) => _refused(name, refusal),
  UpdateEnd_Stopped(stop: UpdateStop.cancelled) => (
    'Update cancelled',
    '$name runs its other copy of Wellfed now. It can feed, and may be older '
        'than the one it ran.',
  ),
  UpdateEnd_Stopped(stop: UpdateStop.hash || UpdateStop.marker) => (
    "Signature didn't check out",
    "The image may be damaged, so it wasn't installed. $name runs its other "
        'copy of Wellfed now, which can feed and may be older.',
  ),
  UpdateEnd_Stopped() || UpdateEnd_Lost() => (
    'Stopped partway',
    "It didn't finish. $name runs its other copy of Wellfed now, which can "
        'feed and may be older than the one it ran.',
  ),
  UpdateEnd_Uncertain() => (
    '$name stopped answering',
    "The update went out, so it may or may not have installed. Wellfed won't "
        'send it again. Look at the firmware once the feeder is back.',
  ),
  UpdateEnd_NotOffered() => ("Couldn't reach $name", 'Nothing was sent, and nothing changed.'),
  UpdateEnd_Unconfirmed() => (
    'Not confirmed yet',
    "$name took the update, but hasn't confirmed the new version. If it "
        "doesn't start properly, it goes to its other copy of Wellfed by itself.",
  ),
  UpdateEnd_WentBack() => (
    '$name is on its other copy',
    "The new version didn't start properly, so it runs its other copy of "
        'Wellfed. That copy can feed, and may be older than the one it ran.',
  ),
  UpdateEnd_WrongBuild() => (
    '$name runs another version',
    "It took the update, but now runs a version this app didn't send: another "
        'update came in between. Check which version it runs before updating again.',
  ),
  UpdateEnd_Failed(:final detail) => ("This $deviceName couldn't send it", 'Nothing was installed. $detail'),
};

/// Why the recovery copy (bank 0) stops an update, and what fixes it. The
/// feeder installs an update from its recovery copy, and only hands over to a
/// copy that started once and checked itself in; one written on the bench
/// has not until it starts.
String recoveryWords(String name, Bank0Blocked reason) => switch (reason) {
  Bank0Blocked.unrecorded =>
    "$name's recovery copy was rewritten and hasn't checked itself in. Start "
        '$name from its recovery copy once (a bench step), then try again.',
  Bank0Blocked.changed =>
    "$name's recovery copy changed since it last checked itself in. Start "
        '$name from its recovery copy once (a bench step), then try again.',
  Bank0Blocked.key =>
    "$name's recovery copy doesn't trust the key that signed this update, so "
        'it could not install it.',
  Bank0Blocked.marker || Bank0Blocked.extent || Bank0Blocked.read =>
    "$name's recovery copy can't be read as working firmware, so it can't "
        'install updates. It needs a bench repair.',
  Bank0Blocked.unknown =>
    "$name's recovery copy won't take this update for a reason this app does "
        'not know. Update this app to read it.',
};

(String, String) _refused(String name, FeederRefusal refusal) => switch (refusal) {
  FeederRefusal_Update(reason: UpdateRefusal_Battery()) => (
    'Plug $name in first',
    "It's running on batteries, and it won't install an update unless it's on "
        'mains power. Nothing changed.',
  ),
  FeederRefusal_Update(reason: UpdateRefusal_Downgrade()) => (
    'Older than what it runs',
    "$name won't go back to an earlier version.",
  ),
  FeederRefusal_Update(
    reason: UpdateRefusal_Manifest() ||
        UpdateRefusal_Key() ||
        UpdateRefusal_Revoked() ||
        UpdateRefusal_Signature() ||
        UpdateRefusal_Statement() ||
        UpdateRefusal_Target(),
  ) =>
    (
      "Signature didn't check out",
      'The image may be damaged, or it was not signed for this feeder. Nothing '
          'was installed.',
    ),
  FeederRefusal_Update(reason: UpdateRefusal_Same()) => ('Nothing to do', '$name already runs this version.'),
  FeederRefusal_Update(reason: UpdateRefusal_Failed()) => (
    'This version failed on $name',
    'It was tried before and did not start properly, so it will not try again.',
  ),
  FeederRefusal_Update(reason: UpdateRefusal_Attempt()) => (
    'Restart $name first',
    'An install already reset it since it was last powered up. Nothing changed.',
  ),
  FeederRefusal_Update(reason: UpdateRefusal_Installing()) => (
    'Another update is under way',
    'Let it finish, then look again. Nothing changed.',
  ),
  FeederRefusal_Update(reason: UpdateRefusal_Pending()) => (
    'An earlier update is still held',
    "One stopped partway, and $name won't take another until it's cancelled. "
        'Nothing is installing now.',
  ),
  FeederRefusal_Update(reason: UpdateRefusal_Fallback(:final reason)) => (
    reason == Bank0Blocked.key ? "The recovery copy can't take it" : 'Updates are blocked',
    '${recoveryWords(name, reason)} Nothing changed.',
  ),
  FeederRefusal_Update(reason: UpdateRefusal_Unknown()) => (
    '$name said no',
    'For a reason this app does not know. Update this app. Nothing changed.',
  ),
  FeederRefusal_Update() => ('$name said no', "It can't install an update right now. Nothing changed."),
  FeederRefusal_Busy() => ("It's feeding", 'Try again when the feed finishes.'),
  FeederRefusal_Forbidden() => ('Owner only', 'Only an owner can update $name. Nothing changed.'),
  _ => ('$name said no', 'Nothing changed.'),
};

/// Where an update ended, with what the feeder runs once it answers. An end only
/// the feeder can settle stays until it has: Done leaves the screen, and coming
/// back shows the same end rather than an offer.
class UpdateEndScreen extends StatelessWidget {
  const UpdateEndScreen({super.key, required this.feeder, required this.ended});

  final FeederController feeder;

  /// The end this screen tells.
  final UpdateEnded ended;

  UpdateEnd get end => ended.end;

  @override
  Widget build(BuildContext context) {
    final UpdateFlow flow = feeder.update;
    final (String title, String body) = updateEndWords(feeder.state.known.name, end);
    final Card? card = feeder.state.card;
    return StepScaffold(
      title: title,
      body: body,
      content: [
        FactList([
          (
            'Running now',
            Flexible(
              child: Text(
                card != null && feeder.state.cardIsCurrent
                    ? 'Bank ${card.bank}, build ${_short(card.build)}'
                    : 'Shown once it answers',
                textAlign: TextAlign.end,
              ),
            ),
          ),
        ]),
        if (ended.stillUnknown) ...[
          const SizedBox(height: Space.s3),
          const Notice(
            tone: Tone.neutral,
            glyph: Glyph.info,
            text: "It hasn't settled yet, or didn't answer. Look again soon.",
          ),
        ],
      ],
      actions: [
        if (_heldEarlier)
          FilledButton(
            onPressed: ended.cancelling
                ? null
                : () async {
                    final FeederReply reply = await flow.cancel();
                    if (!context.mounted) return;
                    if (showProblem(context, reply)) flow.acknowledge();
                  },
            child: const Text('Cancel the earlier update'),
          ),
        if (ended.endUnknown)
          FilledButton(onPressed: () => unawaited(flow.lookAgain()), child: const Text('Look again')),
        OutlinedButton(
          onPressed: () {
            flow.acknowledge();
            if (flow.state.endUnknown) Navigator.pop(context);
          },
          child: const Text('Done'),
        ),
      ],
    );
  }

  /// A stop left its candidate recorded, and only a cancel forgets it.
  bool get _heldEarlier => switch (end) {
    UpdateEnd_Refused(refusal: FeederRefusal_Update(reason: UpdateRefusal_Pending())) => true,
    _ => false,
  };

  static String _short(String build) => build.length > 12 ? build.substring(0, 12) : build;
}
