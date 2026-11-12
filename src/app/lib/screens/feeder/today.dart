import 'dart:async';

import 'package:clock/clock.dart';
import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/screens/feeder/feed_sheet.dart';
import 'package:wellfed/screens/feeder/lights_help.dart';
import 'package:wellfed/screens/feeder/no_time.dart';
import 'package:wellfed/screens/feeder/router_time.dart';
import 'package:wellfed/screens/feeder/today_facts.dart';
import 'package:wellfed/screens/feeder/today_faults.dart';
import 'package:wellfed/screens/feeder/today_timeline.dart';
import 'package:wellfed/screens/home/feeder_card.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/pending.dart';
import 'package:wellfed/widgets/when.dart';

/// How the feeder stands today (7.1): a header with its state, food and lock,
/// what the cat has been given today and every meal of the day, and the facts
/// that need the owner; or what stopped it (7.3). It shows only what the session has read; a fact not read
/// yet says so.
class TodayTab extends StatelessWidget {
  const TodayTab({super.key, required this.feeder});

  final FeederController feeder;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: feeder,
    builder: (context, _) {
      final api.Card? card = feeder.state.card;
      // Only what this session read is the feeder now; an earlier card is history.
      final api.Card? live = feeder.state.cardIsCurrent ? card : null;
      final stopped = live?.blocked == Blocked.fault;

      final List<Widget> facts = [
        if (live != null && !live.clockSet) ...[NoTimeNotice(feeder: feeder), const SizedBox(height: Space.s3)],
        if (live?.time case final time? when live != null && holdsForNoZone(live)) ...[
          NoZoneNotice(feeder: feeder, time: time),
          const SizedBox(height: Space.s3),
        ],
        if (live?.time case final time? when routerGivesNoTime(time)) ...[
          RouterTimeNotice(feeder: feeder, time: time),
          const SizedBox(height: Space.s3),
        ],
        if (stopped) ...[
          TodayStopped(feeder: feeder),
          const SizedBox(height: Space.s3),
        ] else if (card?.fault case final fault?) ...[
          TodayLastFeedFailed(fault: fault),
          const SizedBox(height: Space.s3),
        ],
        if (card != null) ...[
          TodayTimeline(feeder: feeder, card: card),
          const SizedBox(height: Space.s4),
          TodayFacts(feeder: feeder, card: card),
          const SizedBox(height: Space.s3),
        ],
        _LightsLink(),
        if (stopped) ...[
          const SizedBox(height: Space.s3),
          Text(
            'Other faults that show here: the bowl sensor stopped '
            'answering, the outlet is blocked, the motor drew too '
            'much current.',
            style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: context.colors.textMuted),
          ),
        ],
      ];
      // A viewer's account feeds nothing, so it has no Feed to press.
      final Widget? feed = feeder.state.viewer
          ? null
          : FilledButton(
              onPressed: feeder.state.canFeed ? () => feedNow(context, feeder) : null,
              child: Text(stopped ? 'Feed now · unavailable until restart' : 'Feed now'),
            );

      // A desktop window is wide enough for the summary beside the facts,
      // with Feed now under the facts rather than across the whole window.
      if (isDesktop) {
        return LayoutBuilder(
          builder: (context, box) => box.maxWidth < _twoColumns
              ? _OneColumn(feeder: feeder, facts: facts, feed: feed)
              : Row(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Expanded(
                      child: ListView(
                        padding: const EdgeInsets.fromLTRB(Space.s5, Space.s5, Space.s3, Space.s5),
                        children: [_Summary(feeder: feeder)],
                      ),
                    ),
                    SizedBox(
                      width: 360,
                      child: ListView(
                        padding: const EdgeInsets.fromLTRB(Space.s3, Space.s5, Space.s5, Space.s5),
                        children: [
                          if (feed != null) ...[feed, const SizedBox(height: Space.s5)],
                          ...facts,
                        ],
                      ),
                    ),
                  ],
                ),
        );
      }

      return _OneColumn(feeder: feeder, facts: facts, feed: feed);
    },
  );

  /// Wide enough for the summary beside the facts.
  static const double _twoColumns = 720;
}

/// Today on a phone, or a narrow desktop window: the summary over the facts,
/// with Feed now along the bottom.
class _OneColumn extends StatelessWidget {
  const _OneColumn({required this.feeder, required this.facts, required this.feed});

  final FeederController feeder;
  final List<Widget> facts;
  final Widget? feed;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(16, 12, 16, 16),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(
          child: ListView(
            children: [
              _Summary(feeder: feeder),
              const SizedBox(height: Space.s4),
              ...facts,
            ],
          ),
        ),
        if (feed case final feed?) ...[const SizedBox(height: Space.s3), feed],
      ],
    ),
  );
}

class _Summary extends StatelessWidget {
  const _Summary({required this.feeder});

  final FeederController feeder;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final bool online = feeder.state.connection == Connection.online;
    final bool current = feeder.state.cardIsCurrent;
    final api.Card? card = current ? feeder.state.card : null;
    // What the feeder last said, even offline: the picture dims instead.
    final api.Card? read = feeder.state.card;
    final (Color dot, String title) = switch (feeder.state.connection) {
      Connection.looking => (c.textMuted, 'Looking…'),
      Connection.connecting => (c.textMuted, 'Connecting…'),
      Connection.offline => (c.textMuted, "Can't reach it"),
      Connection.otherVersion => (c.warning, 'This feeder needs a different app version'),
      Connection.online when feeder.state.feed.active => (c.accent, 'Feeding'),
      // Open, and its status not read yet: the last session's card says nothing now.
      Connection.online when card == null => (c.textMuted, 'Connecting…'),
      Connection.online when card?.ready ?? false => (c.ok, 'Ready to feed'),
      Connection.online when card?.blocked == Blocked.feeding => (c.accent, 'Feeding'),
      Connection.online when card?.blocked == Blocked.fault => (c.fault, "Can't feed"),
      Connection.online => (c.warning, "Can't feed now"),
    };
    final small = TextStyle(fontSize: Sizes.sm, color: c.textMuted);
    final Blocked? blocked = card?.blocked;
    final bool? locked = read?.locked;

    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        FeederImage(size: 72, dimmed: !online),
        const SizedBox(width: Space.s3),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Container(
                    width: 10,
                    height: 10,
                    decoration: BoxDecoration(color: dot, shape: BoxShape.circle),
                  ),
                  const SizedBox(width: Space.s2),
                  Flexible(
                    child: Text(
                      title,
                      style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
                    ),
                  ),
                ],
              ),
              // Offline, everything under this is what the feeder said then.
              if (feeder.state.cardAt case final DateTime at? when !current)
                Text(lastHeardText(context, at), style: small)
              else if (blocked == Blocked.fault)
                Text('Meals are held until it restarts', style: small)
              else if (blocked != null && blocked != Blocked.feeding)
                Text('${blockedWords(blocked)}.', style: small),
              const SizedBox(height: Space.s2),
              // Centered on one line: the lock's pill sits in a 48 dp tap
              // target, the food's in none.
              Wrap(
                spacing: Space.s2,
                runSpacing: Space.s2,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  _Chip(
                    glyph: Glyph.food,
                    text: switch (read?.food) {
                      FoodLevel.present => 'Food in',
                      FoodLevel.low => 'Food low',
                      FoodLevel.unread || null => 'Food not read',
                      FoodLevel.unknown => 'Food level unknown to this app',
                    },
                    color: read?.food == FoodLevel.low ? c.warning : null,
                  ),
                  // The lock moves on tap, as a switch does, until the card
                  // agrees; a refusal moves it back.
                  Pending<bool>(
                    card: locked ?? false,
                    builder: (context, shownLocked, write) => _Chip(
                      glyph: Glyph.lock,
                      text: switch (locked) {
                        null => 'Lock not read',
                        _ when shownLocked => 'Buttons locked',
                        _ => 'Buttons unlocked',
                      },
                      tapHint: shownLocked ? 'Unlock the buttons' : 'Lock the buttons',
                      onTap: current && locked != null
                          ? () => unawaited(
                              write.ask(!shownLocked, () => feeder.ask(FeederRequest.setLock(locked: !shownLocked))),
                            )
                          : null,
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ],
    );
  }
}

/// A fact in the header, as a pill; one that does something is outlined in
/// the accent and is a 48 dp target.
class _Chip extends StatelessWidget {
  const _Chip({required this.glyph, required this.text, this.color, this.onTap, this.tapHint});

  final Glyph glyph;
  final String text;
  final Color? color;
  final VoidCallback? onTap;
  final String? tapHint;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final Color ink = color ?? (onTap == null ? c.text : c.accent);
    final pill = Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      decoration: BoxDecoration(
        color: c.surface,
        borderRadius: BorderRadius.circular(999),
        border: Border.all(color: onTap == null ? c.line : c.accent.withValues(alpha: 0.6)),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          StrokeIcon(glyph, size: 14, color: ink, weight: 2),
          const SizedBox(width: 6),
          Flexible(
            child: Text(
              text,
              style: TextStyle(fontSize: Sizes.sm, fontWeight: Fonts.medium, color: ink),
            ),
          ),
        ],
      ),
    );
    final VoidCallback? tap = onTap;
    if (tap == null) return Semantics(container: true, child: pill);
    return Semantics(
      container: true,
      button: true,
      onTapHint: tapHint,
      child: InkWell(
        onTap: tap,
        borderRadius: BorderRadius.circular(999),
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 48),
          child: Align(widthFactor: 1, heightFactor: 1, child: pill),
        ),
      ),
    );
  }
}

/// When the card on screen was last current: `Last heard 2:58 PM`, or
/// `Last heard yesterday at 9:00 PM`.
String lastHeardText(BuildContext context, DateTime at) => DateUtils.isSameDay(at, clock.now())
    ? 'Last heard ${clockText(context, at)}'
    : 'Last heard ${atMomentTextMs(context, at.millisecondsSinceEpoch)}';

/// Who last set the feeder's clock, and when.
String clockWords(BuildContext context, api.Card card) {
  if (!card.clockSet) return 'Not set';
  final TimeInfo? time = card.time;
  if (time == null) return 'Set';
  final String by = switch ((time.last, time.ntp)) {
    (TimeSource.ntp, NtpView_Gateway()) => 'Set by your router',
    (TimeSource.ntp, _) => 'Set by a time server',
    (TimeSource.owner || TimeSource.client, _) => 'Set by a paired device',
    (TimeSource.unknown || null, _) => 'Set',
  };
  final PlatformInt64? at = time.lastAtMs;
  return at == null ? by : '$by ${atMomentTextMs(context, at)}';
}

class _LightsLink extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return InkWell(
      onTap: () => unawaited(Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => const LightsHelp()))),
      child: Padding(
        padding: const EdgeInsets.all(Space.s1),
        child: Row(
          children: [
            StrokeIcon(Glyph.info, size: 20, color: c.accent),
            const SizedBox(width: 10),
            Flexible(
              child: Text(
                'What do the lights on the feeder mean?',
                style: TextStyle(fontSize: Sizes.base, fontWeight: Fonts.semibold, color: c.accent),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
