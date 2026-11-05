import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/router.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/layout.dart';

/// What a change is, in a few words, for a sentence.
String writeWords(WriteKind kind) => switch (kind) {
  WriteKind.feed => 'A feed',
  WriteKind.cancel => 'Stopping a feed',
  WriteKind.settings => 'A settings change',
  WriteKind.lock => 'The button lock',
  WriteKind.desiccant => 'The desiccant reset',
  WriteKind.plans => 'A schedule change',
  WriteKind.wifi => 'A Wi-Fi change',
  WriteKind.play => 'Playing the meal call',
  WriteKind.clock => 'Setting the clock',
  WriteKind.invite => 'An invite',
  WriteKind.revoke => 'Removing a phone',
  WriteKind.reboot => 'A restart',
  WriteKind.calibration => 'The portion size',
  WriteKind.call => 'A meal call change',
  WriteKind.update => 'A firmware update',
};

/// Changes other than a feed that went out and never said how they ended.
/// Each stays on the way it went: Check only reads that way, and nothing is
/// sent again. The owner puts one away once they have looked.
class UnconfirmedWrites extends StatelessWidget {
  const UnconfirmedWrites({super.key, required this.feeder});

  final FeederController feeder;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: feeder,
    builder: (context, _) {
      final List<PendingWrite> held = feeder.state.unconfirmed;
      if (held.isEmpty) return const SizedBox.shrink();
      return Padding(
        padding: const EdgeInsets.fromLTRB(Space.s4, Space.s3, Space.s4, 0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final PendingWrite pending in held) ...[
              Notice(
                tone: Tone.warning,
                lead: '${writeWords(pending.kind)} may not have reached the feeder.',
                text: 'Check asks the way it went, and sends nothing again.',
              ),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  TextButton(onPressed: () => unawaited(feeder.checkWrite(pending.key)), child: const Text('Check')),
                  TextButton(onPressed: () => feeder.putAwayWrite(pending.key), child: const Text('Put away')),
                ],
              ),
            ],
          ],
        ),
      );
    },
  );
}

/// The way this phone reaches a feeder now, under its name.
class PathLine extends StatelessWidget {
  const PathLine({super.key, required this.path});

  final RoutePath? path;

  @override
  Widget build(BuildContext context) => Text(switch (path) {
    RoutePath.server => 'Through the home server',
    RoutePath.direct => 'Directly',
    null => 'No way to it now',
  }, style: TextStyle(fontSize: Sizes.xs, color: context.colors.textMuted));
}
