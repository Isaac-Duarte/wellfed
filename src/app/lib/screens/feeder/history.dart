import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/screens/feeder/feed_detail.dart';
import 'package:wellfed/screens/feeder/today_faults.dart';
import 'package:wellfed/screens/home/feeder_card.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/when.dart';

part 'history_words.dart';

enum _Filter { all, feeds, problems, changes }

bool _shown(_Filter filter, HistoryBody body) => switch (filter) {
  _Filter.all => body is! HistoryBody_Other,
  _Filter.feeds => body is HistoryBody_Fed || body is HistoryBody_Missed,
  _Filter.problems => switch (body) {
    HistoryBody_Fed(:final result) => result.outcome != FeedOutcome.done || result.confirmed != result.requested,
    HistoryBody_Food(:final level) => level == FoodLevel.low,
    // Setup mode is the owner's own choice, not something gone wrong.
    HistoryBody_FeedingBlocked(:final reason) => reason != null && reason != Blocked.setup,
    HistoryBody_Missed() || HistoryBody_DispenseRefused() || HistoryBody_Lost() || HistoryBody_Gap() => true,
    _ => false,
  },
  _Filter.changes => switch (body) {
    HistoryBody_Lock() ||
    HistoryBody_SettingsChanged() ||
    HistoryBody_WifiChanged() ||
    HistoryBody_PlansChanged() ||
    HistoryBody_Skipped() ||
    HistoryBody_Unskipped() ||
    HistoryBody_Joined() ||
    HistoryBody_UpdateKeyAdded() ||
    HistoryBody_Revoked() ||
    HistoryBody_ClockSet() => true,
    _ => false,
  },
};

/// Who a client is, from the list the feeder gives; a key it no longer lists
/// keeps a generic name, since the event carries only its fingerprint.
String whoText(FeederController feeder, FedBy by) => switch (by) {
  FedBy_Panel() => 'The Dispense button',
  FedBy_Client(:final fingerprint) => feeder.clientName(fingerprint),
  FedBy_Feeder() => 'The feeder',
  FedBy_Plan() => 'A meal',
  FedBy_Unknown() => 'Unknown',
};

/// What the feeder's history keeps (9.1): the ring, newest first. It is the
/// feeder's own record, read by the session; 9.3's longer history needs a
/// server, and is not here.
///
/// Each day is one card, headed by how its meals went. Feeds and problems
/// lead with a tinted badge; changes and routine notes stay quiet, and a run
/// of the same change folds into one row, so five settings saves don't push
/// the day's feeds off the screen.
///
/// With the owner's server, `longHistory` opens what the server kept, which
/// reaches back further than the feeder's ring; that screen draws its
/// `entries` here, newest first, with `empty` while it has none.
class HistoryTab extends StatefulWidget {
  const HistoryTab({super.key, required this.feeder, this.longHistory, this.entries, this.empty});

  final FeederController feeder;
  final VoidCallback? longHistory;
  final List<HistoryEntry>? entries;
  final String? empty;

  @override
  State<HistoryTab> createState() => _HistoryTabState();
}

class _HistoryTabState extends State<HistoryTab> {
  /// The route's storage keeps the choice while the feeder screen stays open,
  /// as a tab away disposes this state.
  static const _filterKey = 'history-filter';

  late _Filter _filter =
      PageStorage.maybeOf(context)?.readState(context, identifier: _filterKey) as _Filter? ?? _Filter.all;

  /// Folded runs the owner opened, by their newest entry's sequence number.
  final Set<BigInt> _open = {};

  void _choose(_Filter filter) {
    PageStorage.maybeOf(context)?.writeState(context, filter, identifier: _filterKey);
    setState(() => _filter = filter);
  }

  void _toggle(BigInt seq) => setState(() => _open.contains(seq) ? _open.remove(seq) : _open.add(seq));

  @override
  void initState() {
    super.initState();
    unawaited(widget.feeder.loadClients());
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.feeder,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final FeederController feeder = widget.feeder;
      final List<_Day> days = _days(context, feeder, widget.entries ?? feeder.state.history.reversed, _filter);
      final VoidCallback? longHistory = feeder.state.serverId == null ? null : widget.longHistory;

      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
            child: Row(
              children: [
                for (final (name, filter) in const [
                  ('All', _Filter.all),
                  ('Feeds', _Filter.feeds),
                  ('Problems', _Filter.problems),
                  ('Changes', _Filter.changes),
                ]) ...[
                  ChoiceChip(
                    label: Text(name),
                    showCheckmark: false,
                    selected: _filter == filter,
                    onSelected: (_) => _choose(filter),
                  ),
                  const SizedBox(width: Space.s2),
                ],
                if (longHistory != null) TextButton(onPressed: longHistory, child: const Text('Full history')),
              ],
            ),
          ),
          Expanded(
            child: days.isEmpty
                ? Center(
                    child: Text(
                      widget.empty ??
                          (feeder.state.historyRead
                              ? 'Nothing here yet.'
                              : feeder.state.connection == Connection.online
                              ? 'Reading the history…'
                              : "Can't read the history right now."),
                      style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                    ),
                  )
                : ListView(
                    padding: EdgeInsets.fromLTRB(16, 4, 16, 16 + MediaQuery.paddingOf(context).bottom),
                    children: [
                      for (final day in days) _DayCard(day: day, feeder: feeder, open: _open, toggle: _toggle),
                    ],
                  ),
          ),
        ],
      );
    },
  );
}

/// One day of history: its heading, how its meals went, and its rows.
class _Day {
  _Day(this.heading);

  final String heading;
  final List<_Run> runs = [];
  int fed = 0;
  int missed = 0;
}

/// One row: an entry, or a run of the same quiet change folded together,
/// newest first.
class _Run {
  _Run(this.words, HistoryEntry first) : entries = [first];

  final _Words words;
  final List<HistoryEntry> entries;

  HistoryEntry get newest => entries.first;
}

List<_Day> _days(BuildContext context, FeederController feeder, Iterable<HistoryEntry> newestFirst, _Filter filter) {
  final days = <_Day>[];
  _Day? last;
  for (final entry in newestFirst) {
    final PlatformInt64? at = entry.atMs;
    // An entry with no time stays under the day before it; a first entry
    // with none gets a heading of its own.
    final String? heading = at == null ? null : dayText(DateTime.fromMillisecondsSinceEpoch(at));
    final _Day day = last == null || (heading != null && heading != last.heading) ? _Day(heading ?? 'Earlier') : last;
    if (!identical(day, last)) days.add(last = day);

    // The day's tally counts every meal, whatever the filter shows.
    final HistoryBody body = entry.body;
    if (body is HistoryBody_Fed) day.fed++;
    if (body is HistoryBody_Missed) day.missed++;
    if (!_shown(filter, body)) continue;

    final _Words words = _describe(context, feeder, body);
    final _Run? run = day.runs.isEmpty ? null : day.runs.last;
    // Only quiet rows fold: each feed and each problem keeps its own row.
    if (run != null && !words.loud && run.words == words) {
      run.entries.add(entry);
    } else {
      day.runs.add(_Run(words, entry));
    }
  }
  return [
    for (final day in days)
      if (day.runs.isNotEmpty) day,
  ];
}

class _DayCard extends StatelessWidget {
  const _DayCard({required this.day, required this.feeder, required this.open, required this.toggle});

  final _Day day;
  final FeederController feeder;
  final Set<BigInt> open;
  final void Function(BigInt seq) toggle;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final String tally = [
      if (day.fed > 0) '${day.fed} ${day.fed == 1 ? 'feed' : 'feeds'}',
      if (day.missed > 0) '${day.missed} missed',
    ].join(' · ');

    final rows = <Widget>[];
    for (final _Run run in day.runs) {
      final bool opened = open.contains(run.newest.seq);
      rows.add(
        _EntryRow(
          entry: run.newest,
          words: run.words,
          first: rows.isEmpty,
          times: run.entries.length,
          opened: opened,
          onTap: switch (run.newest.body) {
            HistoryBody_Fed(:final FeedResult result) => () => unawaited(
              Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => FeedDetail(feeder: feeder, result: result),
                ),
              ),
            ),
            _ when run.entries.length > 1 => () => toggle(run.newest.seq),
            _ => null,
          },
        ),
      );
      if (opened) rows.add(_FoldedTimes(run.entries));
    }

    return Padding(
      padding: const EdgeInsets.only(top: Space.s3),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(Space.s1, 0, Space.s1, Space.s2),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    day.heading,
                    style: TextStyle(fontSize: Sizes.base, fontWeight: Fonts.semibold, color: c.text),
                  ),
                ),
                if (tally.isNotEmpty)
                  Text(
                    tally,
                    style: TextStyle(fontSize: Sizes.sm, color: day.missed > 0 ? c.warning : c.textMuted),
                  ),
              ],
            ),
          ),
          Panel(child: Column(children: rows)),
        ],
      ),
    );
  }
}

class _EntryRow extends StatelessWidget {
  const _EntryRow({
    required this.entry,
    required this.words,
    required this.first,
    required this.times,
    required this.opened,
    required this.onTap,
  });

  final HistoryEntry entry;
  final _Words words;
  final bool first;

  /// How many entries the row stands for; more than one is a folded run.
  final int times;
  final bool opened;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final PlatformInt64? at = entry.atMs;
    final String note = [if (words.note.isNotEmpty) words.note, if (times > 1) '$times times'].join(' · ');

    return InkWell(
      onTap: onTap,
      child: Container(
        constraints: const BoxConstraints(minHeight: 60),
        padding: const EdgeInsets.all(Space.s3),
        decoration: BoxDecoration(
          border: first ? null : Border(top: BorderSide(color: c.line)),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            _Badge(words: words),
            const SizedBox(width: Space.s3),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    words.title,
                    style: TextStyle(
                      fontSize: Sizes.base,
                      fontWeight: words.loud ? Fonts.semibold : Fonts.medium,
                      color: c.text,
                    ),
                  ),
                  if (note.isNotEmpty)
                    Padding(
                      padding: const EdgeInsets.only(top: 2),
                      child: Text(
                        note,
                        style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: Space.s2),
            // The newest time of a run; opening it lists the rest.
            if (at != null)
              Text(
                clockTextMs(context, at),
                style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
              ),
            if (onTap != null)
              Padding(
                padding: const EdgeInsets.only(left: Space.s1),
                child: RotatedBox(
                  quarterTurns: times > 1 ? (opened ? 3 : 1) : 0,
                  child: StrokeIcon(Glyph.chevron, size: 16, color: c.textMuted),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// A feed or a problem gets a tinted disc; a quiet change a ringed line icon.
class _Badge extends StatelessWidget {
  const _Badge({required this.words});

  final _Words words;

  @override
  Widget build(BuildContext context) => Container(
    width: 32,
    height: 32,
    alignment: Alignment.center,
    decoration: words.loud
        ? BoxDecoration(color: words.color.withValues(alpha: 0.16), shape: BoxShape.circle)
        : BoxDecoration(
            shape: BoxShape.circle,
            border: Border.all(color: context.colors.line),
          ),
    child: StrokeIcon(words.glyph, size: 16, color: words.color, weight: words.loud ? 2 : 1.8),
  );
}

/// Every time in an opened run, under its row.
class _FoldedTimes extends StatelessWidget {
  const _FoldedTimes(this.entries);

  final List<HistoryEntry> entries;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(56, 0, Space.s3, Space.s3),
    child: Align(
      alignment: Alignment.centerLeft,
      child: Text(
        [
          for (final entry in entries)
            if (entry.atMs case final at?) clockTextMs(context, at) else 'Time not known',
        ].join(', '),
        style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: context.colors.textMuted),
      ),
    ),
  );
}
