import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/feeder/shell.dart' show confirmRestart;
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/when.dart';

/// What a feed's reason comes to for an owner, with the time when known.
String faultTitle(BuildContext context, FeedReason reason, int? atMs) {
  final String what = switch (reason) {
    FeedReason.jammed => 'The wheel jammed',
    FeedReason.outletBlocked => 'The outlet is blocked',
    FeedReason.overcurrent => 'The motor drew too much current',
    FeedReason.sensor => 'A sensor stopped answering',
    FeedReason.noFeedback || FeedReason.feedbackStream => 'The feeder lost track of its wheel',
    FeedReason.markStuck => 'The wheel sensor stuck',
    FeedReason.power => 'The power went out during a feed',
    FeedReason.interrupted => 'The feeder restarted during a feed',
    FeedReason.output => 'The motor did not respond',
    FeedReason.clock => 'The clock jumped during a feed',
    FeedReason.observationGap => 'The feeder could not watch its motor',
    FeedReason.timeout => 'A feed ran past its limit',
    FeedReason.cancelled => 'A feed was stopped',
    FeedReason.done => 'A feed ended',
  };
  return atMs == null ? what : '$what ${atMomentTextMs(context, atMs)}';
}

/// How far the fault's feed got, when its history has come in.
String? _stoppedAfter(api.FaultView fault) {
  final api.FeedResult? fed = fault.result;
  if (fed == null) return null;
  return fed.uncertain
      ? 'It stopped somewhere between ${fed.confirmed} and ${fed.atMost} of '
            '${fed.requested} ${portionWord(fed.requested)}.'
      : 'It stopped after ${deliveredText(fed)} ${portionWord(fed.requested)}.';
}

/// A fault holds feeding until a restart (7.3).
class TodayStopped extends StatelessWidget {
  const TodayStopped({super.key, required this.feeder});

  final FeederController feeder;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final api.FaultView? fault = feeder.state.card?.fault;
    final String? after = fault == null ? null : _stoppedAfter(fault);
    final String title = fault == null
        ? 'A fault stopped feeding'
        : faultTitle(context, fault.reason, fault.result?.atMs);
    final stuck = fault?.reason == FeedReason.jammed;
    final String detail = [?after, if (stuck) 'Something may be stuck in the chute.'].join(' ');
    final muted = TextStyle(fontSize: Sizes.base, height: 1.5, color: c.textMuted);
    final step = TextStyle(fontSize: Sizes.base, height: 1.6, color: c.text);

    return Container(
      padding: const EdgeInsets.all(Space.s4),
      decoration: BoxDecoration(color: c.faultSoft, borderRadius: BorderRadius.circular(Radii.card)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            title,
            style: TextStyle(fontSize: 17, fontWeight: Fonts.semibold, color: c.text),
          ),
          if (detail.isNotEmpty) ...[const SizedBox(height: Space.s2), Text(detail, style: muted)],
          const SizedBox(height: Space.s2),
          for (final (i, line) in const [
            'Unplug the feeder and take out the batteries.',
            'Empty the hopper and clear the chute above the bowl.',
            'Plug it back in. It restarts and can feed again.',
          ].indexed)
            Text('${i + 1}. $line', style: step),
          const SizedBox(height: Space.s3),
          OutlinedButton(
            onPressed: () => unawaited(confirmRestart(context, feeder)),
            child: const Text('Restart it from here'),
          ),
        ],
      ),
    );
  }
}

/// A feed ended on a fault, and the feeder can feed again.
class TodayLastFeedFailed extends StatelessWidget {
  const TodayLastFeedFailed({super.key, required this.fault});

  final api.FaultView fault;

  @override
  Widget build(BuildContext context) => Notice(
    tone: Tone.fault,
    lead: '${faultTitle(context, fault.reason, fault.result?.atMs)}.',
    text:
        '${_stoppedAfter(fault) ?? 'The last feed did not finish.'} Look at '
        'the bowl before you feed again.',
  );
}
