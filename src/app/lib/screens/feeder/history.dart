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
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/when.dart';

part 'history_rows.dart';
part 'history_words.dart';

enum _Filter { all, feeds, problems, changes }

bool _shown(_Filter filter, HistoryBody body) => switch (filter) {
  _Filter.all => body is! HistoryBody_Other,
  _Filter.feeds => body is HistoryBody_Fed || body is HistoryBody_Missed,
  _Filter.problems => switch (body) {
    HistoryBody_Fed(:final result) => fedKind(result) != FedKind.full,
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

/// What the feeder's history keeps: the ring, newest first. It is the
/// feeder's own record, read by the session.
///
/// Each day is one card, headed by how its meals went. Feeds and problems
/// lead with a tinted badge; changes and routine notes stay quiet, and a run
/// of the same change folds into one row, so five settings saves don't push
/// the day's feeds off the screen.
class HistoryTab extends StatefulWidget {
  const HistoryTab({super.key, required this.feeder});

  final FeederController feeder;

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

  /// The feed beside the list, by its entry's sequence number (unique in the
  /// history, where a feed number need not be), on a window wide enough for
  /// both.
  BigInt? _beside;

  /// Wide enough for the list and a feed's detail side by side.
  static const double _twoPanes = 760;

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
      final List<_Day> days = _days(context, feeder, feeder.state.history.reversed, _filter);

      // A desktop shows a feed beside the list rather than over it, so the
      // list keeps its place while the owner reads down it.
      return LayoutBuilder(
        builder: (context, box) {
          final bool panes = isDesktop && box.maxWidth >= _twoPanes;
          final HistoryEntry? beside = panes ? _entry(days, _beside) : null;
          final list = _HistoryList(
            feeder: feeder,
            days: days,
            filter: _filter,
            choose: _choose,
            open: _open,
            toggle: _toggle,
            shown: beside?.seq,
            openFeed: (seq, result) => panes
                ? setState(() => _beside = seq)
                : unawaited(
                    Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) => FeedDetail(feeder: feeder, result: result),
                      ),
                    ),
                  ),
          );
          if (!panes) return list;

          return Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Expanded(child: list),
              VerticalDivider(width: 1, color: c.line),
              SizedBox(
                width: 340,
                child: _FeedPane(
                  feeder: feeder,
                  result: switch (beside?.body) {
                    HistoryBody_Fed(:final FeedResult result) => result,
                    _ => null,
                  },
                ),
              ),
            ],
          );
        },
      );
    },
  );

  /// The feed entry `seq` among what the list shows, or none once a filter
  /// or the ring's turn leaves it out.
  static HistoryEntry? _entry(List<_Day> days, BigInt? seq) {
    if (seq == null) return null;
    for (final _Day day in days) {
      for (final _Run run in day.runs) {
        if (run.newest.seq == seq && run.newest.body is HistoryBody_Fed) return run.newest;
      }
    }
    return null;
  }
}
