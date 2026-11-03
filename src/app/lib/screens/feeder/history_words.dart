part of 'history.dart';

/// What a row says about its entry. A loud row is a feed or a problem.
typedef _Words = ({Glyph glyph, Color color, String title, String note, bool loud});

_Words _describe(BuildContext context, FeederController feeder, HistoryBody body) {
  final ThemeTokens c = context.colors;
  String who(FedBy by) => whoText(feeder, by);
  String asker(FedBy by) => _asker(feeder, by);
  _Words loud(Glyph glyph, Color color, String title, String note) =>
      (glyph: glyph, color: color, title: title, note: note, loud: true);
  _Words quiet(Glyph glyph, String title, String note) =>
      (glyph: glyph, color: c.textMuted, title: title, note: note, loud: false);

  return switch (body) {
    HistoryBody_Fed(:final FeedResult result) when result.uncertain => loud(
      Glyph.warning,
      c.warning,
      'Fed between ${result.confirmed} and ${result.atMost}',
      result.by is FedBy_Unknown ? 'Interrupted · not repeated' : '${asker(result.by)} · not repeated',
    ),
    HistoryBody_Fed(:final FeedResult result)
        when result.outcome == FeedOutcome.done && result.confirmed == result.requested =>
      loud(Glyph.check, c.ok, 'Fed ${fedAmountText(result)}', asker(result.by)),
    HistoryBody_Fed(:final FeedResult result) => loud(
      Glyph.warning,
      c.warning,
      'Fed ${fedAmountText(result)}',
      '${asker(result.by)} · ${faultTitle(context, result.reason, null)}',
    ),
    HistoryBody_Missed(:final PlatformInt64 dueMs, :final MissedReason reason) => loud(
      Glyph.warning,
      c.warning,
      '${clockTextMs(context, dueMs)} meal skipped',
      _missed(reason),
    ),
    HistoryBody_Restarted() => quiet(Glyph.power, 'Started up', ''),
    HistoryBody_Lock(:final bool locked, :final FedBy by) => quiet(
      Glyph.lock,
      locked ? 'Buttons locked' : 'Buttons unlocked',
      who(by),
    ),
    HistoryBody_SettingsChanged(:final FedBy by) => quiet(Glyph.sliders, 'Settings changed', who(by)),
    HistoryBody_WifiChanged(:final FedBy by) => quiet(Glyph.wifi, 'Wi-Fi changed', who(by)),
    HistoryBody_PlansChanged(:final FedBy by) => quiet(Glyph.clock, 'Meals changed', who(by)),
    HistoryBody_Joined(:final String fingerprint) => quiet(Glyph.add, 'Someone joined', feeder.clientName(fingerprint)),
    HistoryBody_UpdateKeyAdded(:final String fingerprint) => quiet(
      Glyph.key,
      'Update key added',
      feeder.clientName(fingerprint),
    ),
    HistoryBody_Revoked(:final String fingerprint, :final FedBy by) => quiet(
      Glyph.minus,
      '${feeder.clientName(fingerprint)} removed',
      who(by),
    ),
    HistoryBody_ClockSet(:final FedBy by) => quiet(
      Glyph.clock,
      'Clock set',
      by is FedBy_Feeder ? 'From its time server' : who(by),
    ),
    HistoryBody_Lost(:final int count) => loud(
      Glyph.warning,
      c.warning,
      "Some events weren't kept",
      '$count could not be recorded',
    ),
    HistoryBody_Gap() => quiet(Glyph.info, "Some events here weren't kept", ''),
    HistoryBody_FeedingBlocked(reason: Blocked.setup) => quiet(
      Glyph.info,
      'Not ready to feed',
      blockedWords(Blocked.setup),
    ),
    HistoryBody_FeedingBlocked(reason: final Blocked reason?) => loud(
      Glyph.warning,
      c.warning,
      'Not ready to feed',
      blockedWords(reason),
    ),
    HistoryBody_FeedingBlocked(reason: null) => loud(Glyph.check, c.ok, 'Ready to feed again', ''),
    HistoryBody_MealSoon(:final int minute, :final int portions) => quiet(
      Glyph.clock,
      'Meal coming',
      '${minuteText(context, minute)} · ${portionsText(portions)}',
    ),
    HistoryBody_Supply(:final bool? adapter) => quiet(Glyph.power, switch (adapter) {
      true => 'Back on mains',
      false => 'On batteries',
      null => 'Power changed',
    }, ''),
    HistoryBody_DispenseRefused(:final Blocked reason) => quiet(
      Glyph.warning,
      'A press was refused',
      blockedWords(reason),
    ),
    HistoryBody_Food(:final FoodLevel level) =>
      level == FoodLevel.low
          ? loud(Glyph.food, c.warning, 'Food read low', '')
          : quiet(Glyph.food, level == FoodLevel.present ? 'Food in the hopper' : 'Food not read', ''),
    HistoryBody_Other() => quiet(Glyph.info, '', ''),
  };
}

/// Who asked for a feed. A meal's feed says only that it was the schedule:
/// the event names the slot, and the slot's time today may not be the time
/// it had then. The row already shows when it ran.
String _asker(FeederController feeder, FedBy by) => switch (by) {
  FedBy_Plan() => 'Scheduled meal',
  FedBy_Client() => 'Feed now · ${whoText(feeder, by)}',
  _ => whoText(feeder, by),
};

String _missed(MissedReason reason) => switch (reason) {
  MissedReason_Late() => 'The feeder was off or had no time',
  MissedReason_Blocked(:final reason) => switch (reason) {
    Blocked.battery => 'The batteries were too low',
    _ => blockedWords(reason),
  },
  MissedReason_Interrupted() => 'The feeder stopped first',
};
