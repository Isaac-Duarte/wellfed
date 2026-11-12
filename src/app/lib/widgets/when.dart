import 'package:clock/clock.dart';
import 'package:flutter/material.dart';

import 'package:wellfed/src/rust/api/model.dart' show FeedOutcome;
import 'package:wellfed/src/rust/api/session.dart';

/// A moment as this phone shows the time of day.
String clockText(BuildContext context, DateTime at) => _text(context, TimeOfDay.fromDateTime(at));

/// A moment in milliseconds since the epoch, by this phone's clock.
String clockTextMs(BuildContext context, int ms) => clockText(context, DateTime.fromMillisecondsSinceEpoch(ms));

/// A moment that may be old: `9:00 PM` today, `Yesterday 9:00 PM`, or
/// `Fri 30 Oct 9:00 PM`. A bare time the next morning would read as today's.
String momentTextMs(BuildContext context, int ms, {DateTime? now}) {
  final at = DateTime.fromMillisecondsSinceEpoch(ms);
  final String time = clockText(context, at);
  return DateUtils.isSameDay(at, now ?? clock.now()) ? time : '${dayText(at, now: now)} $time';
}

/// The same moment for the middle of a sentence: `at 9:00 PM`,
/// `yesterday at 9:00 PM`, or `on Fri 30 Oct at 9:00 PM`.
String atMomentTextMs(BuildContext context, int ms, {DateTime? now}) {
  final at = DateTime.fromMillisecondsSinceEpoch(ms);
  final String time = clockText(context, at);
  return switch (dayText(at, now: now)) {
    'Today' => 'at $time',
    'Yesterday' => 'yesterday at $time',
    'Tomorrow' => 'tomorrow at $time',
    final String day => 'on $day at $time',
  };
}

/// A minute of the feeder's local day, as a plan keeps it.
String minuteText(BuildContext context, int minute) =>
    _text(context, TimeOfDay(hour: minute ~/ 60 % 24, minute: minute % 60));

/// `in 26 min`, `in 2 h 5 min`, or `now` once it is due.
String untilText(DateTime now, int atMs) {
  final int minutes = (atMs - now.millisecondsSinceEpoch) ~/ 60000;
  if (minutes < 1) return 'now';
  if (minutes < 60) return 'in $minutes min';
  final int rest = minutes % 60;
  return rest == 0 ? 'in ${minutes ~/ 60} h' : 'in ${minutes ~/ 60} h $rest min';
}

String portionsText(int n) => '$n ${portionWord(n)}';

/// `portion` or `portions`, for a count said elsewhere in the sentence.
String portionWord(int n) => n == 1 ? 'portion' : 'portions';

/// A weight in grams, without its unit: `9.8`, or `11` when it is whole.
/// Every screen says a weight this one way, so a feed reads the same before
/// and after it runs.
String gramsText(int mg) => mg % 1000 == 0 ? '${mg ~/ 1000}' : (mg / 1000).toStringAsFixed(1);

String _text(BuildContext context, TimeOfDay time) =>
    MaterialLocalizations.of(context)
        .formatTimeOfDay(time, alwaysUse24HourFormat: MediaQuery.alwaysUse24HourFormatOf(context));

/// `2 of 2`, or a range for an uncertain feed, never one number.
String deliveredText(FeedResult fed) =>
    fed.uncertain ? '${fed.confirmed} to ${fed.atMost} of ${fed.requested}' : '${fed.confirmed} of ${fed.requested}';

/// How much a feed gave, in portions: "2 portions" when it gave all it was
/// asked for, otherwise the count against the request ("1 of 2 portions").
String fedAmountText(FeedResult fed) => fed.uncertain || fed.confirmed != fed.requested
    ? '${deliveredText(fed)} ${portionWord(fed.requested)}'
    : portionsText(fed.confirmed);

/// How a feed ended, as every screen marks it.
enum FedKind {
  /// It ran to its end and gave every portion asked.
  full,

  /// It gave fewer, or ended some other way: a fault, a cancel.
  short,

  /// Its count is not known: it is never repeated.
  unsure,
}

/// The one rule for [FedKind], so a feed reads alike on every screen.
FedKind fedKind(FeedResult fed) => fed.uncertain
    ? FedKind.unsure
    : fed.outcome == FeedOutcome.done && fed.confirmed == fed.requested
    ? FedKind.full
    : FedKind.short;

const _dayNames = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const _monthNames = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/// `Today`, `Yesterday`, or `Fri 30 Oct`, by this phone's calendar.
String dayText(DateTime at, {DateTime? now}) {
  final DateTime today = now ?? clock.now();
  // Calendar days, in UTC: two local midnights across a spring-forward day are
  // 23 hours apart, which is 0 whole days.
  final int days = DateTime.utc(
    today.year,
    today.month,
    today.day,
  ).difference(DateTime.utc(at.year, at.month, at.day)).inDays;
  return switch (days) {
    0 => 'Today',
    1 => 'Yesterday',
    -1 => 'Tomorrow',
    _ => '${_dayNames[at.weekday - 1]} ${at.day} ${_monthNames[at.month - 1]}',
  };
}
