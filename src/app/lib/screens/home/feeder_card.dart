import 'dart:async';

import 'package:clock/clock.dart';
import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/feeder/feed_sheet.dart';
import 'package:wellfed/screens/feeder/no_time.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/src/rust/api/feeder.dart' show PlansView;
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/when.dart';

class FeederCard extends StatelessWidget {
  const FeederCard({super.key, required this.controller, this.compact = false});

  final FeederController controller;

  /// One of several: a row with a small Feed button.
  final bool compact;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) => compact
        ? Builder(
            builder: (context) {
              final ThemeTokens c = context.colors;
              final String? line = _line(context);
              final String? why = _why();
              return Panel(
                padding: const EdgeInsets.fromLTRB(10, 14, 14, 14),
                onTap: controller.state.feed is FeedIdle
                    ? () => _open(context)
                    : () => showFeedSheet(context, controller),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Row(
                      children: [
                        FeederImage(size: 72, dimmed: _unreachable),
                        const SizedBox(width: 10),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                controller.state.known.name,
                                style: TextStyle(fontSize: 17, fontWeight: Fonts.semibold, color: c.text),
                              ),
                              const SizedBox(height: 6),
                              _Chips(controller: controller),
                              if (line != null) ...[
                                const SizedBox(height: 6),
                                Text(
                                  line,
                                  style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                                ),
                              ],
                            ],
                          ),
                        ),
                        const SizedBox(width: Space.s2),
                        FilledButton(
                          style: FilledButton.styleFrom(
                            minimumSize: const Size(0, 44),
                            padding: const EdgeInsets.symmetric(horizontal: 14),
                            textStyle: const TextStyle(
                              fontFamily: Fonts.family,
                              fontSize: Sizes.base,
                              fontWeight: Fonts.semibold,
                            ),
                          ),
                          onPressed: controller.state.canFeed ? () => feedNow(context, controller) : null,
                          child: const Text('Feed'),
                        ),
                      ],
                    ),
                    if (why != null) ...[
                      const SizedBox(height: Space.s3),
                      Padding(
                        padding: const EdgeInsets.only(left: 6),
                        child: Text(
                          why,
                          style: TextStyle(fontSize: Sizes.sm, height: 1.45, color: c.textMuted),
                        ),
                      ),
                    ],
                  ],
                ),
              );
            },
          )
        : Builder(
            builder: (context) {
              final ThemeTokens c = context.colors;
              final api.Card? card = controller.state.card;
              final List<Widget> notices = _notices(context);
              return Panel(
                padding: const EdgeInsets.all(Space.s4),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    InkWell(
                      onTap: () => _open(context),
                      child: Row(
                        children: [
                          FeederImage(size: 96, dimmed: _unreachable),
                          const SizedBox(width: 14),
                          Expanded(
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  controller.state.known.name,
                                  style: TextStyle(fontSize: 22, fontWeight: Fonts.semibold, color: c.text),
                                ),
                                const SizedBox(height: Space.s2),
                                _Chips(controller: controller),
                              ],
                            ),
                          ),
                          StrokeIcon(Glyph.chevron, size: 22, color: c.textMuted),
                        ],
                      ),
                    ),
                    if (card != null) ...[
                      const SizedBox(height: Space.s4),
                      Builder(
                        builder: (context) {
                          final (String power, String battery) = switch ((card.adapter, card.batteryPercent)) {
                            (true, final int percent?) => ('Mains', 'Battery $percent%'),
                            (true, null) => ('Mains', 'Plugged in'),
                            (false, final int percent?) => ('Battery', '$percent%, Wi-Fi off'),
                            (false, null) => ('Battery', 'Wi-Fi off'),
                            (null, _) => ('Unknown', 'Not read yet'),
                          };
                          final api.NextMealView? next = card.nextMeal;
                          final api.FeedResult? last = card.lastFed;
                          final PlatformInt64? lastAt = last?.atMs;
                          // The next meal leads across the card. Under it, equal heights, as a day
                          // or "Time unknown" takes the last-fed tile's value onto a second line.
                          return Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              if (next == null)
                                _Tile('Next meal', '—', _noNext(card.plans), lead: true)
                              else
                                _Tile(
                                  'Next meal',
                                  minuteText(context, next.minute),
                                  portionsText(next.portions),
                                  lead: true,
                                ),
                              const SizedBox(height: Space.s2),
                              IntrinsicHeight(
                                child: Row(
                                  crossAxisAlignment: CrossAxisAlignment.stretch,
                                  children: [
                                    Expanded(
                                      child: last == null
                                          ? const _Tile('Last fed', '—', 'Not read yet')
                                          : _Tile(
                                              'Last fed',
                                              lastAt == null ? 'Time unknown' : momentTextMs(context, lastAt),
                                              fedAmountText(last),
                                            ),
                                    ),
                                    const SizedBox(width: Space.s2),
                                    Expanded(child: _Tile('Power', power, battery)),
                                  ],
                                ),
                              ),
                            ],
                          );
                        },
                      ),
                    ],
                    for (final notice in notices) ...[const SizedBox(height: 14), notice],
                    const SizedBox(height: Space.s4),
                    FilledButton(
                      onPressed: controller.state.canFeed ? () => feedNow(context, controller) : null,
                      child: const Text('Feed now'),
                    ),
                    if (controller.state.feed is! FeedIdle) ...[
                      const SizedBox(height: Space.s1),
                      TextButton(
                        onPressed: () => showFeedSheet(context, controller),
                        child: Text(_feedLink(controller.state.feed)),
                      ),
                    ],
                  ],
                ),
              );
            },
          ),
  );

  /// Why the card names no next meal. Only meals read and none enabled mean
  /// there is none; meals still being read, or held for want of the time,
  /// say so rather than read as an empty schedule.
  static String _noNext(PlansView? plans) {
    if (plans == null) return 'Reading…';
    final bool any = plans.plans.any((plan) => plan?.enabled ?? false);
    return any ? 'Meals held' : 'No meal set';
  }

  void _open(BuildContext context, [int tab = todayTab]) =>
      unawaited(context.push('/feeder/${controller.state.known.identity}?tab=$tab'));

  bool get _unreachable => controller.state.connection == Connection.offline;

  /// The live card: the status read in the session now open.
  api.Card? get _live => controller.state.connection == Connection.online ? controller.state.card : null;

  List<Widget> _notices(BuildContext context) {
    final api.Card? card = controller.state.card;
    final api.Card? live = _live;
    final String? why = _why();
    return [
      if (live != null && !live.clockSet) NoTimeNotice(feeder: controller),
      if (live?.blocked case final blocked? when blocked != Blocked.feeding)
        Notice(tone: Tone.warning, text: '${blockedWords(blocked)}.'),
      if (card?.fault case final fault?)
        Notice(
          tone: Tone.fault,
          lead: fault.outcome == FeedOutcome.uncertain ? "The last feed didn't finish." : 'The last feed failed.',
          text:
              'The feeder stopped without recording how much came out. Look '
              'at the bowl before you feed again.',
        ),
      if (card?.food == FoodLevel.low)
        Notice(
          tone: Tone.fault,
          lead: _lowLead(context, card),
          text:
              'Refill it soon. A meal can run short from here, so check the '
              'bowl if your pet seems hungry.',
        ),
      if (why != null) Notice(tone: Tone.neutral, text: why),
    ];
  }

  /// The hopper's last reading, with its time only when the last feed took
  /// it. A reading at rest carries no time, and the last feed may have read
  /// food long before it.
  static String _lowLead(BuildContext context, api.Card? card) {
    final api.FeedResult? last = card?.lastFed;
    final PlatformInt64? at = last?.hopper == FoodLevel.low ? last?.atMs : null;
    return at == null ? 'Food is low.' : 'Food read low ${atMomentTextMs(context, at)}.';
  }

  /// The compact card's second line.
  String? _line(BuildContext context) {
    final FeedState feed = controller.state.feed;
    if (feed is FeedRunning) return 'Started from this phone';
    if (controller.state.connection != Connection.offline) {
      final api.NextMealView? next = controller.state.card?.nextMeal;
      return next == null
          ? null
          : 'Next ${minuteText(context, next.minute)} · '
                '${portionsText(next.portions)}';
    }
    final DateTime? at = controller.state.cardAt;
    if (at == null) return 'Not heard from yet';
    final int minutes = clock.now().difference(at).inMinutes;
    return minutes < 1 ? 'Last heard just now' : 'Last heard $minutes min ago';
  }

  /// Why the feeder is out of reach, when the card can tell.
  String? _why() {
    if (controller.state.card?.adapter == false) {
      return 'The power adapter is out, so it turned Wi-Fi off to save the '
          "batteries. Meals still run on time. You'll see it again when "
          "it's plugged back in.";
    }
    if (controller.state.connection == Connection.offline) {
      return "This phone can't reach it. Check that it's plugged in, the "
          'Wi-Fi light is steady, and this phone is on the same Wi-Fi.';
    }
    return null;
  }

  static String _feedLink(FeedState feed) => switch (feed) {
    FeedSending() || FeedRunning() => 'Feeding…',
    FeedEnded(:final api.FeedResult result) => 'Last feed: ${result.words}',
    FeedRefused() => 'The feeder did not feed',
    FeedUnanswered() || FeedAdrift() => 'A feed may have run',
    FeedNotSent() => 'The last feed was not sent',
    FeedIdle() => '',
  };
}

String blockedWords(Blocked reason) => switch (reason) {
  Blocked.starting => 'Starting up',
  Blocked.journal => 'Its feed record could not open; restart the feeder',
  Blocked.fault => 'Stopped on a fault until it restarts',
  Blocked.resolving => 'Settling a feed a restart interrupted',
  Blocked.feeding => 'A feed is running',
  Blocked.joining => 'Joining Wi-Fi',
  Blocked.updating => 'Installing an update',
  Blocked.setup => 'In setup mode',
  Blocked.unserved => 'This firmware does not feed',
  Blocked.battery => 'Battery too low to feed',
  Blocked.restarting => 'Restarting',
};

/// A sunken fact: a label, the value, and a note under it.
class _Tile extends StatelessWidget {
  const _Tile(this.label, this.value, this.note, {this.lead = false});

  final String label;
  final String value;
  final String note;

  /// The tile the card leads with: its value set larger.
  final bool lead;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final small = TextStyle(fontSize: Sizes.xs, color: c.textMuted);
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(color: c.sunken, borderRadius: BorderRadius.circular(Radii.icon)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(label, style: small),
          const SizedBox(height: 2),
          Text(
            value,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(fontSize: lead ? Sizes.lg : Sizes.md, fontWeight: Fonts.semibold, color: c.text),
          ),
          Text(note, maxLines: 1, overflow: TextOverflow.ellipsis, style: small),
        ],
      ),
    );
  }
}

/// The status line of a feeder's card.
class _Chips extends StatelessWidget {
  const _Chips({required this.controller});

  final FeederController controller;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final FeederState state = controller.state;
    final api.Card? card = state.card;
    final api.Card? live = state.connection == Connection.online ? card : null;
    final FeedState feed = state.feed;

    final StatusChip status = switch (state.connection) {
      Connection.looking => StatusChip('Looking…', color: c.textMuted),
      Connection.connecting => StatusChip('Connecting…', color: c.textMuted),
      Connection.offline => StatusChip("Can't reach it", color: c.textMuted),
      Connection.online when feed is FeedRunning => StatusChip(
        'Feeding ${feed.portions == 1 ? '1 portion' : '${feed.portions} portions'}',
        color: c.accent,
      ),
      Connection.online when live?.ready ?? false => StatusChip('Ready', color: c.ok),
      Connection.online when live?.blocked == Blocked.feeding => StatusChip('Feeding', color: c.accent),
      Connection.online => StatusChip("Can't feed now", color: c.warning),
    };

    return Wrap(
      spacing: 14,
      runSpacing: 6,
      children: [
        status,
        if (card != null) _FoodChip(card.food),
        if (card?.adapter == false) StatusChip('On battery', color: c.warning, glyph: Glyph.battery),
        if (live != null && !live.clockSet) StatusChip('No time', color: c.warning, glyph: Glyph.clock),
      ],
    );
  }
}

/// How much food the feeder reads.
class _FoodChip extends StatelessWidget {
  const _FoodChip(this.food);

  final FoodLevel food;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final (String text, Color color) = switch (food) {
      FoodLevel.present => ('Food in', c.text),
      FoodLevel.low => ('Food low', c.fault),
      FoodLevel.unread => ('Food not read', c.textMuted),
    };
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        FoodDot(
          present: switch (food) {
            FoodLevel.present => true,
            FoodLevel.low => false,
            FoodLevel.unread => null,
          },
        ),
        const SizedBox(width: 6),
        Text(
          text,
          style: TextStyle(fontSize: Sizes.sm, fontWeight: Fonts.semibold, color: color),
        ),
      ],
    );
  }
}
