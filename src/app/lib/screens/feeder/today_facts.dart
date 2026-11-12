import 'package:flutter/material.dart';
import 'package:wellfed/screens/feeder/today.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/replies.dart';
import 'package:wellfed/widgets/when.dart';

/// The facts under the drawing. Only what needs the owner shows by itself:
/// running on batteries or low on them, a weak signal, no clock, a desiccant
/// due. The rest folds into one line that opens to the full list.
class TodayFacts extends StatefulWidget {
  const TodayFacts({super.key, required this.feeder, required this.card});

  final FeederController feeder;
  final api.Card card;

  @override
  State<TodayFacts> createState() => _TodayFactsState();
}

enum _Fact { power, wifi, clock, desiccant }

/// Under this the batteries won't carry the feeder through a long outage.
const _lowBattery = 20;

/// The signal the Wi-Fi row calls weak, in dBm.
const _weakSignal = -72;

class _TodayFactsState extends State<TodayFacts> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final api.Card card = widget.card;
    final unusual = <_Fact>{
      if (card.adapter == false || (card.batteryPercent ?? 100) < _lowBattery) _Fact.power,
      if (card.rssi < _weakSignal) _Fact.wifi,
      if (!card.clockSet) _Fact.clock,
      if (card.desiccant case Desiccant_Remaining(due: true)) _Fact.desiccant,
    };
    final List<_Fact> shown = _open ? _Fact.values : _Fact.values.where(unusual.contains).toList();
    // The facts speak of the feeder now only once this session has read them.
    final bool online = widget.feeder.state.cardIsCurrent;

    return Panel(
      child: Column(
        children: [
          for (final (i, fact) in shown.indexed)
            _FactRow(fact, feeder: widget.feeder, card: card, online: online, first: i == 0),
          _More(
            open: _open,
            allFine: unusual.isEmpty,
            live: online,
            first: shown.isEmpty,
            onTap: () => setState(() => _open = !_open),
          ),
        ],
      ),
    );
  }
}

class _FactRow extends StatelessWidget {
  const _FactRow(this.fact, {required this.feeder, required this.card, required this.online, required this.first});

  final _Fact fact;
  final FeederController feeder;
  final api.Card card;
  final bool online;
  final bool first;

  @override
  Widget build(BuildContext context) => switch (fact) {
    _Fact.power => _Row(Glyph.power, 'Power', first: first, value: _Value(_power(card))),
    _Fact.wifi => _Row(Glyph.wifi, 'Wi-Fi', first: first, value: _Value(_signal(card.rssi))),
    _Fact.clock => _Row(Glyph.clock, 'Clock', first: first, value: _Value(clockWords(context, card))),
    _Fact.desiccant => _Row(
      Glyph.droplet,
      'Desiccant',
      first: first,
      value: _DesiccantValue(feeder: feeder, card: card, online: online),
    ),
  };

  static String _power(api.Card card) => switch ((card.adapter, card.batteryPercent)) {
    (true, final int percent?) => 'Mains · batteries $percent%',
    (true, null) => 'Mains',
    (false, final int percent?) => 'Batteries $percent%',
    (false, null) => 'Batteries',
    (null, _) => 'Not read yet',
  };

  static String _signal(int rssi) => switch (rssi) {
    >= -60 => 'Strong',
    >= _weakSignal => 'Fair',
    _ => 'Weak',
  };
}

/// The fold: what it holds when closed, and a way to close it again. Offline
/// it speaks of when the feeder was last heard, never of now.
class _More extends StatelessWidget {
  const _More({
    required this.open,
    required this.allFine,
    required this.live,
    required this.first,
    required this.onTap,
  });

  final bool open;
  final bool allFine;

  /// The card is the feeder's word now; otherwise it is from when Today's
  /// "Last heard" line says.
  final bool live;
  final bool first;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final String words = switch ((open, allFine, live)) {
      (true, _, _) => 'Show less',
      (false, true, true) => 'Power, Wi-Fi and clock look fine',
      (false, false, true) => 'Everything else looks fine',
      (false, true, false) => 'Power, Wi-Fi and clock were fine then',
      (false, false, false) => 'Everything else was fine then',
    };
    final (Glyph glyph, Color color) = switch ((open, live)) {
      (true, _) => (Glyph.minus, c.textMuted),
      (false, true) => (Glyph.check, c.ok),
      (false, false) => (Glyph.clock, c.textMuted),
    };
    return Semantics(
      container: true,
      button: true,
      expanded: open,
      child: InkWell(
        onTap: onTap,
        child: Container(
          constraints: const BoxConstraints(minHeight: 48),
          padding: const EdgeInsets.symmetric(horizontal: Space.s4, vertical: 8),
          decoration: BoxDecoration(
            border: first ? null : Border(top: BorderSide(color: c.line)),
          ),
          child: Row(
            children: [
              StrokeIcon(glyph, size: 22, color: color),
              const SizedBox(width: 14),
              Expanded(
                child: Text(words, style: TextStyle(fontSize: 15, color: c.text)),
              ),
              RotatedBox(
                quarterTurns: open ? 3 : 1,
                child: StrokeIcon(Glyph.chevron, size: 20, color: c.textMuted),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// One fact, one accessibility node: without the container every row's text
/// merges into the list item and reads out the whole panel.
class _Row extends StatelessWidget {
  const _Row(this.glyph, this.label, {required this.value, this.first = false});

  final Glyph glyph;
  final String label;
  final Widget value;
  final bool first;

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
            Text(label, style: TextStyle(fontSize: 15, color: c.text)),
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
      Desiccant_Remaining(:final days) => (overdueText(-days), c.warning),
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
          TextButton(onPressed: widget.online && !_asking ? _changed : null, child: const Text('Mark changed')),
      ],
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
