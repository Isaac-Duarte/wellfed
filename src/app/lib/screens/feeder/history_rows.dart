part of 'history.dart';

/// The filters over the days of history.
class _HistoryList extends StatelessWidget {
  const _HistoryList({
    required this.feeder,
    required this.days,
    required this.filter,
    required this.choose,
    required this.open,
    required this.toggle,
    required this.shown,
    required this.openFeed,
  });

  final FeederController feeder;
  final List<_Day> days;
  final _Filter filter;
  final void Function(_Filter filter) choose;
  final Set<BigInt> open;
  final void Function(BigInt seq) toggle;

  /// The feed entry shown beside the list, by sequence number, marked in it.
  final BigInt? shown;
  final void Function(BigInt seq, FeedResult result) openFeed;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
          child: Row(
            children: [
              for (final (name, choice) in const [
                ('All', _Filter.all),
                ('Feeds', _Filter.feeds),
                ('Problems', _Filter.problems),
                ('Changes', _Filter.changes),
              ]) ...[
                ChoiceChip(
                  label: Text(name),
                  showCheckmark: false,
                  selected: filter == choice,
                  onSelected: (_) => choose(choice),
                ),
                const SizedBox(width: Space.s2),
              ],
            ],
          ),
        ),
        Expanded(
          child: days.isEmpty
              ? Center(
                  child: Text(
                    feeder.state.historyRead
                        ? 'Nothing here yet.'
                        : feeder.state.connection == Connection.online
                        ? 'Reading the history…'
                        : "Can't read the history right now.",
                    style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                  ),
                )
              : ListView(
                  padding: EdgeInsets.fromLTRB(16, 4, 16, 16 + MediaQuery.paddingOf(context).bottom),
                  children: [
                    for (final day in days)
                      _DayCard(day: day, open: open, toggle: toggle, openFeed: openFeed, shown: shown),
                  ],
                ),
        ),
      ],
    );
  }
}

/// The feed beside the list on a desktop, or a word on how to show one.
class _FeedPane extends StatelessWidget {
  const _FeedPane({required this.feeder, required this.result});

  final FeederController feeder;
  final FeedResult? result;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return switch (result) {
      final FeedResult result => FeedDetailBody(
        feeder: feeder,
        result: result,
        heading: Text(
          feedWhen(context, result),
          style: TextStyle(fontSize: Sizes.md, fontWeight: Fonts.semibold, color: c.text),
        ),
      ),
      null => Center(
        child: Padding(
          padding: const EdgeInsets.all(Space.s5),
          child: Text(
            'Choose a feed to see how it went.',
            textAlign: TextAlign.center,
            style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
          ),
        ),
      ),
    };
  }
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
  const _DayCard({
    required this.day,
    required this.open,
    required this.toggle,
    required this.openFeed,
    required this.shown,
  });

  final _Day day;
  final Set<BigInt> open;
  final void Function(BigInt seq) toggle;
  final void Function(BigInt seq, FeedResult result) openFeed;

  /// The feed entry shown beside the list, by sequence number, marked in it.
  final BigInt? shown;

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
          selected: run.newest.seq == shown,
          onTap: switch (run.newest.body) {
            HistoryBody_Fed(:final FeedResult result) => () => openFeed(run.newest.seq, result),
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
    required this.selected,
    required this.onTap,
  });

  final HistoryEntry entry;
  final _Words words;
  final bool first;

  /// How many entries the row stands for; more than one is a folded run.
  final int times;
  final bool opened;

  /// Shown beside the list, on a desktop.
  final bool selected;
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
          color: selected ? c.accentSoft : null,
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
