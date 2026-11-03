import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/screens/feeder/feed_sheet.dart';
import 'package:wellfed/screens/feeder/lights_help.dart';
import 'package:wellfed/screens/feeder/no_time.dart';
import 'package:wellfed/screens/feeder/router_time.dart';
import 'package:wellfed/screens/feeder/today_faults.dart';
import 'package:wellfed/screens/home/feeder_card.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/replies.dart';
import 'package:wellfed/widgets/when.dart';

/// How the feeder stands today, its facts and the lock (7.1), or what
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
                  if (card != null) ...[_Facts(feeder: feeder, card: card), const SizedBox(height: Space.s3)],
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
    final api.Card? card = feeder.state.connection == Connection.online ? feeder.state.card : null;
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
    final api.NextMealView? next = feeder.state.card?.nextMeal;
    final api.FeedResult? last = feeder.state.card?.lastFed;
    final Blocked? blocked = card?.blocked;

    return Row(
      children: [
        FeederImage(size: 120, dimmed: feeder.state.connection == Connection.offline),
        const SizedBox(width: Space.s4),
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
              const SizedBox(height: Space.s1),
              if (blocked == Blocked.fault)
                Text('Meals are held until it restarts', style: small)
              else if (blocked != null && blocked != Blocked.feeding)
                Text('${blockedWords(blocked)}.', style: small),
              if (next != null)
                Text(
                  'Next meal ${minuteText(context, next.minute)} · '
                  '${portionsText(next.portions)}',
                  style: small,
                ),
              if (last != null) Text(_lastFed(context, last), style: small),
            ],
          ),
        ),
      ],
    );
  }

  static String _lastFed(BuildContext context, api.FeedResult last) {
    final PlatformInt64? at = last.atMs;
    final when = at == null ? '' : ' ${momentTextMs(context, at)} ·';
    return 'Last fed$when ${deliveredText(last)}';
  }
}

class _Facts extends StatelessWidget {
  const _Facts({required this.feeder, required this.card});

  final FeederController feeder;
  final api.Card card;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    // A time only when the last feed took this reading: one at rest has none.
    final api.FeedResult? fed = card.lastFed;
    final PlatformInt64? last = fed?.hopper == card.food ? fed?.atMs : null;
    final at = last == null ? '' : ' ${atMomentTextMs(context, last)}';
    final (String food, Color foodColor) = switch (card.food) {
      FoodLevel.present => ('In the hopper$at', c.food),
      FoodLevel.low => ('Low$at', c.fault),
      FoodLevel.unread => ('Not read yet', c.textMuted),
    };
    final online = feeder.state.connection == Connection.online;

    return Panel(
      child: Column(
        children: [
          _Row(
            Glyph.food,
            'Food',
            first: true,
            value: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Container(
                  width: 8,
                  height: 8,
                  decoration: BoxDecoration(color: foodColor, shape: BoxShape.circle),
                ),
                const SizedBox(width: 6),
                Flexible(child: _Value(food)),
              ],
            ),
          ),
          _Row(Glyph.power, 'Power', value: _Value(_power(card))),
          _Row(Glyph.wifi, 'Wi-Fi', value: _Value(_signal(card.rssi))),
          _Row(Glyph.clock, 'Clock', value: _Value(clockWords(context, card))),
          _Row(
            Glyph.droplet,
            'Desiccant',
            value: _DesiccantValue(feeder: feeder, card: card, online: online),
          ),
          _LockRow(feeder: feeder, card: card, online: online),
        ],
      ),
    );
  }

  static String _power(api.Card card) => switch ((card.adapter, card.batteryPercent)) {
    (true, final int percent?) => 'Mains · batteries $percent%',
    (true, null) => 'Mains',
    (false, final int percent?) => 'Batteries $percent%',
    (false, null) => 'Batteries',
    (null, _) => 'Not read yet',
  };

  static String _signal(int rssi) => switch (rssi) {
    >= -60 => 'Strong',
    >= -72 => 'Fair',
    _ => 'Weak',
  };
}

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

/// One fact, one accessibility node. Without the container every row's text
/// and the lock's switch merge into the list item, as one switch that reads
/// out the whole panel.
class _Row extends StatelessWidget {
  const _Row(this.glyph, this.label, {required this.value, this.first = false, this.valueNamed = false});

  final Glyph glyph;
  final String label;
  final Widget value;
  final bool first;

  /// The value carries `label` itself, so the words are not read out twice.
  final bool valueNamed;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Semantics(
      container: true,
      child: Container(
        constraints: const BoxConstraints(minHeight: 48),
        padding: const EdgeInsets.symmetric(horizontal: Space.s4, vertical: 8),
        decoration: BoxDecoration(
          border: first ? null : Border(top: BorderSide(color: c.line)),
        ),
        child: Row(
          children: [
            StrokeIcon(glyph, size: 22, color: c.textMuted),
            const SizedBox(width: 14),
            ExcludeSemantics(
              excluding: valueNamed,
              child: Text(label, style: TextStyle(fontSize: 15, color: c.text)),
            ),
            const SizedBox(width: Space.s3),
            Expanded(
              child: Align(alignment: Alignment.centerRight, child: value),
            ),
          ],
        ),
      ),
    );
  }
}

class _DesiccantValue extends StatefulWidget {
  const _DesiccantValue({required this.feeder, required this.card, required this.online});

  final FeederController feeder;
  final api.Card card;
  final bool online;

  @override
  State<_DesiccantValue> createState() => _DesiccantValueState();
}

class _DesiccantValueState extends State<_DesiccantValue> {
  bool _asking = false;

  Future<void> _changed() async {
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(const FeederRequest.resetDesiccant());
    if (!mounted) return;
    setState(() => _asking = false);
    showProblem(context, reply);
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final Desiccant desiccant = widget.card.desiccant;
    final (String words, Color color) = switch (desiccant) {
      Desiccant_Unset() => ('Not started', c.textMuted),
      Desiccant_Unknown() => ('Unknown until the clock is set', c.textMuted),
      Desiccant_Remaining(days: 0) => ('Change today', c.warning),
      Desiccant_Remaining(:final days) when days > 0 => (
        days == 1 ? 'Change in 1 day' : 'Change in $days days',
        c.text,
      ),
      Desiccant_Remaining(:final days) => (days == -1 ? '1 day overdue' : '${-days} days overdue', c.warning),
    };
    final bool offer = switch (desiccant) {
      Desiccant_Unset() => true,
      Desiccant_Remaining(:final due) => due,
      Desiccant_Unknown() => false,
    };

    return Wrap(
      alignment: WrapAlignment.end,
      crossAxisAlignment: WrapCrossAlignment.center,
      spacing: Space.s2,
      children: [
        Text(
          words,
          style: TextStyle(fontSize: Sizes.base, color: color),
        ),
        if (offer)
          TextButton(onPressed: widget.online && !_asking ? _changed : null, child: const Text('Changed today')),
      ],
    );
  }
}

class _LockRow extends StatefulWidget {
  const _LockRow({required this.feeder, required this.card, required this.online});

  final FeederController feeder;
  final api.Card card;
  final bool online;

  @override
  State<_LockRow> createState() => _LockRowState();
}

class _LockRowState extends State<_LockRow> {
  bool _asking = false;

  Future<void> _set(bool locked) async {
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(FeederRequest.setLock(locked: locked));
    if (!mounted) return;
    setState(() => _asking = false);
    showProblem(context, reply);
  }

  @override
  Widget build(BuildContext context) {
    final bool? locked = widget.card.locked;
    const name = 'Lock the buttons';
    return _Row(
      Glyph.lock,
      name,
      valueNamed: true,
      value: Semantics(
        container: true,
        label: name,
        child: Switch(
          // What the feeder last said, not what was asked: an answer moves it.
          value: locked ?? false,
          onChanged: widget.online && locked != null && !_asking ? _set : null,
        ),
      ),
    );
  }
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

class _Value extends StatelessWidget {
  const _Value(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Text(
    text,
    textAlign: TextAlign.end,
    style: TextStyle(fontSize: Sizes.base, color: context.colors.text),
  );
}
