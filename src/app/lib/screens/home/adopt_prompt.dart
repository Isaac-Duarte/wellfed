import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/server/server_words.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/adopt.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/layout.dart';

/// A feeder this phone paired before the home server, which the server does
/// not list: offered to put on it, so the household and the server reach it.
class AdoptPrompt extends StatelessWidget {
  const AdoptPrompt({super.key, required this.feeder, required this.flow});

  final KnownFeeder feeder;
  final AdoptFlow flow;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: flow,
    builder: (context, _) {
      final AdoptStatus? status = flow.state[feeder.identity];
      final String name = feeder.name;
      final (Tone tone, String text) = switch (status) {
        null => (
          Tone.neutral,
          'The home server does not have $name yet. Put it there so the household reaches it too.',
        ),
        AdoptRunning() => (Tone.neutral, 'Putting $name on the home server…'),
        AdoptDone() => (Tone.neutral, '$name is on the home server.'),
        AdoptUnclaimed(:final error) => (
          Tone.warning,
          '$name is on the server, but this $deviceName is not yours there yet. ${serverErrorWords(error)}',
        ),
        AdoptUnsettled(:final error) => (
          Tone.warning,
          switch (error.kind) {
            ServerErrorKind.outcomeUnknown =>
              "The server didn't hear back from $name in time, and $name may still appear on it. "
                  "Don't put it on again: Check reads the server's list and sends nothing new.",
            ServerErrorKind.feederBusy =>
              '$name is still being put on the server. Check reads the feeder list and sends nothing new.',
            _ => "The server's answer was lost. Check reads the server's list and sends nothing new.",
          },
        ),
        AdoptNoInvite() => (Tone.warning, "$name didn't answer the invite. Check it is online, then try again."),
        AdoptFailed(:final error) => (Tone.fault, serverErrorWords(error)),
      };
      final String? action = switch (status) {
        null || AdoptNoInvite() || AdoptFailed() => 'Put on the server',
        AdoptUnclaimed() => 'Try again',
        AdoptUnsettled() => 'Check',
        AdoptRunning() || AdoptDone() => null,
      };
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Notice(tone: tone, text: text),
          if (action != null)
            Align(
              alignment: AlignmentDirectional.centerEnd,
              child: TextButton(onPressed: () => unawaited(flow.adopt(feeder)), child: Text(action)),
            ),
          const SizedBox(height: Space.s3),
        ],
      );
    },
  );
}
