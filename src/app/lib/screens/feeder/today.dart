import 'dart:async';

import 'package:clock/clock.dart';
import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/screens/feeder/feed_sheet.dart';
import 'package:wellfed/screens/feeder/lights_help.dart';
import 'package:wellfed/screens/feeder/no_time.dart';
import 'package:wellfed/screens/feeder/router_time.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/screens/feeder/today_facts.dart';
import 'package:wellfed/screens/feeder/today_faults.dart';
import 'package:wellfed/screens/home/feeder_card.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/feeder_drawing.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/pending.dart';
import 'package:wellfed/widgets/when.dart';

/// How the feeder stands today (7.1): the drawing with its food, last feed
/// and lock, the next meal, and the facts that need the owner; or what
/// stopped it (7.3). It shows only what the session has read; a fact not read
/// yet says so.
class TodayTab extends StatelessWidget {
  const TodayTab({super.key, required this.feeder});

  final FeederController feeder;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: feeder,
    builder: (context, _) {
      final api.Card? card = feeder.state.card;
      final api.Card? live = feeder.state.connection == Connection.online ? card : null;
      final stopped = live?.blocked == Blocked.fault;

      return Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(
              child: ListView(
                children: [
                  _Summary(feeder: feeder),
                  const SizedBox(height: Space.s3),
                  if (live != null && !live.clockSet) ...[
                    NoTimeNotice(feeder: feeder),
                    const SizedBox(height: Space.s3),
                  ],
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
                  if (card != null) ...[TodayFacts(feeder: feeder, card: card), const SizedBox(height: Space.s3)],
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
                ],
              ),
            ),
            const SizedBox(height: Space.s3),
            FilledButton(
              onPressed: feeder.state.canFeed ? () => feedNow(context, feeder) : null,
              child: Text(stopped ? 'Feed now · unavailable until restart' : 'Feed now'),
            ),
          ],
        ),
      );
    },
  );
}

class _Summary extends StatelessWidget {
  const _Summary({required this.feeder});

  final FeederController feeder;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final bool online = feeder.state.connection == Connection.online;
    final api.Card? card = online ? feeder.state.card : null;
    // What the feeder last said, even offline: the drawing dims instead.
    final api.Card? read = feeder.state.card;
    final (Color dot, String title) = switch (feeder.state.connection) {
      Connection.looking => (c.textMuted, 'Looking…'),
      Connection.connecting => (c.textMuted, 'Connecting…'),
      Connection.offline => (c.textMuted, "Can't reach it"),
      Connection.online when feeder.state.feed.active => (c.accent, 'Feeding'),
      Connection.online when card?.ready ?? false => (c.ok, 'Ready to feed'),
      Connection.online when card?.blocked == Blocked.feeding => (c.accent, 'Feeding'),
      Connection.online when card?.blocked == Blocked.fault => (c.fault, "Can't feed"),
      Connection.online => (c.warning, "Can't feed now"),
    };
    final small = TextStyle(fontSize: Sizes.base, color: c.textMuted);
    final Blocked? blocked = card?.blocked;

    final (String food, Color? foodColor, FoodShown shown) = switch (read?.food) {
      FoodLevel.present => ('Food in', null, FoodShown.full),
      FoodLevel.low => ('Food low', c.warning, FoodShown.low),
      FoodLevel.unread || null => ('Food not read', c.textMuted, FoodShown.none),
    };
    final bool? locked = read?.locked;
    // The lock label moves on tap, as a switch does, until the card agrees; a
    // refusal moves it back.
    List<Callout> callouts(bool shownLocked, PendingWrite<bool> write) => [
      Callout(anchor: 'window', side: CalloutSide.right, lines: [food], color: foodColor),
      Callout(
        anchor: 'button_lock',
        side: CalloutSide.left,
        lines: [
          switch (locked) {
            null => 'Lock not read',
            _ when shownLocked => 'Buttons locked',
            _ => 'Buttons unlocked',
          },
        ],
        color: locked == null ? c.textMuted : null,
        onTap: online && locked != null
            ? () => unawaited(write.ask(!shownLocked, () => feeder.ask(FeederRequest.setLock(locked: !shownLocked))))
            : null,
        tapHint: shownLocked ? 'Unlock the buttons' : 'Lock the buttons',
      ),
      _lastFed(context, read?.lastFed),
    ];

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
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
        if (feeder.state.cardAt case final DateTime at? when !online)
          Text(lastHeardText(context, at), style: small)
        else if (blocked == Blocked.fault)
          Text('Meals are held until it restarts', style: small)
        else if (blocked != null && blocked != Blocked.feeding)
          Text('${blockedWords(blocked)}.', style: small),
        const SizedBox(height: Space.s3),
        Pending<bool>(
          card: locked ?? false,
          builder: (context, shownLocked, write) => FeederDrawing(
            height: 280,
            callouts: callouts(shownLocked, write),
            food: shown,
            dimmed: !online,
            label: 'Drawing of the feeder',
          ),
        ),
        if (read != null) ...[const SizedBox(height: Space.s3), _NextMeal(card: read)],
      ],
    );
  }

  /// The bowl's callout: when, and how much went out. A short or unsure feed
  /// says its count against what was asked, never one number. Three short
  /// lines, so it fits beside the drawing on a phone.
  static Callout _lastFed(BuildContext context, api.FeedResult? last) {
    if (last == null) {
      return const Callout(anchor: 'bowl', side: CalloutSide.right, lines: ['Last feed', 'not read yet']);
    }
    final PlatformInt64? at = last.atMs;
    final String when = at == null ? '' : momentTextMs(context, at);
    final String amount = fedAmountText(last);
    return Callout(
      anchor: 'bowl',
      side: CalloutSide.right,
      lines: ['Last fed', if (when.isNotEmpty) when, amount],
      summary: when.isEmpty ? 'Last fed · $amount' : 'Last fed $when · $amount',
    );
  }
}

/// A space a line never breaks at, so "2 portions" wraps as one.
const _keep = '\u00a0';

/// The next meal, the line the owner opens the tab for, or a way to add one.
class _NextMeal extends StatelessWidget {
  const _NextMeal({required this.card});

  final api.Card card;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final style = TextStyle(fontSize: Sizes.md, fontWeight: Fonts.semibold, color: c.text);
    final api.NextMealView? next = card.nextMeal;
    final TabController? tabs = DefaultTabController.maybeOf(context);

    return Row(
      children: [
        StrokeIcon(Glyph.clock, size: 22, color: next == null ? c.textMuted : c.accent),
        const SizedBox(width: Space.s3),
        Expanded(
          child: next == null
              ? Text('No meal set', style: style.copyWith(color: c.textMuted))
              : Text(
                  'Next meal ${minuteText(context, next.minute)} · '
                  '${untilText(clock.now(), next.atMs)} · ${portionsText(next.portions).replaceAll(' ', _keep)}',
                  style: style,
                ),
        ),
        // Only inside the feeder's tabs, where the schedule is one tap away.
        if (next == null && tabs != null)
          TextButton(onPressed: () => tabs.animateTo(scheduleTab), child: const Text('Add a meal')),
      ],
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
    (null, _) => 'Set',
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
