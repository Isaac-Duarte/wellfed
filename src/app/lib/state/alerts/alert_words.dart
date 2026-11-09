import 'package:wellfed/platform/alert_ports.dart' show NotificationAsk;
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/model.dart';

/// What a notice says: a title and a sentence or two.
typedef NoticeWords = ({String title, String body});

/// `notice` from the feeder called `feeder`, in the words a notification and
/// the alert list use. The feeder decided the alert; these only say it.
NoticeWords noticeWords(NoticeView notice, String feeder) {
  final NoticeDetail detail = notice.detail;
  return switch (notice.kind) {
    NoticeKind.mealFailed => (
      title: '$feeder: a meal failed',
      body: '${_portions(detail)} ${_failedWhy(detail)} Check the bowl and the feeder.',
    ),
    NoticeKind.mealUncertain => (
      title: '$feeder: a meal may not have dropped',
      body: "It can't say how much dropped. Look at the bowl before you feed again.",
    ),
    NoticeKind.mealMissed => (
      title: '$feeder: a meal was missed',
      body: switch (detail) {
        NoticeDetail_Meal(missed: MissedReason_Blocked()) => "It couldn't feed when the meal was due.",
        NoticeDetail_Meal(missed: MissedReason_Interrupted()) => 'It stopped before the meal began.',
        _ => 'It was off or had no time when the meal was due.',
      },
    ),
    NoticeKind.mealNoRecord => (
      title: '$feeder: a meal has no record',
      body: 'A meal is well past due and the feeder has no record of it. Check the bowl.',
    ),
    NoticeKind.noClock => (
      title: "$feeder doesn't know the time",
      body: 'Meals wait until it does. Open the app at home to set its clock.',
    ),
    NoticeKind.cannotFeed => (
      title: "$feeder can't feed",
      body: "It hasn't been able to feed for a while. Open the app to see why.",
    ),
    NoticeKind.plansStuck => (
      title: "$feeder can't read its meals",
      body: 'Its meal plans have been out of reach for a while, so meals may not run.',
    ),
    NoticeKind.foodLow => (title: '$feeder is low on food', body: 'Top up the hopper soon.'),
    NoticeKind.foodLikelyOut => (
      title: '$feeder is probably out of food',
      body: switch (detail) {
        NoticeDetail_FoodLikelyOut(:final int confirmed) =>
          '$confirmed portions have gone since it read low. Top up the hopper.',
        _ => 'Top up the hopper.',
      },
    ),
    NoticeKind.desiccantDue => (
      title: "$feeder's desiccant is due",
      body: 'Change the desiccant pack, then reset its count in the app.',
    ),
    NoticeKind.batteryBelowFloor => (
      title: "$feeder's batteries are low",
      body: 'Replace them, or meals stop in a power cut.',
    ),
    NoticeKind.powerWasOut => (
      title: '$feeder lost power',
      body: 'It ran on its batteries for a while. Check that the adapter is plugged in.',
    ),
    NoticeKind.updateFailed => (
      title: "$feeder's update stopped",
      body: 'The feeder kept the firmware it had. Try the update again.',
    ),
    NoticeKind.rolledBack => (
      title: '$feeder went back to its recovery firmware',
      body: 'The firmware it was running stopped. Open the app to update it again.',
    ),
    NoticeKind.repeatedRestarts => (
      title: '$feeder keeps restarting',
      body: 'It restarted several times in a few hours. Check its power.',
    ),
    NoticeKind.clientJoined => (
      title: 'Someone joined $feeder',
      body: switch (detail) {
        NoticeDetail_Client(role: ClientRole.owner) => 'A new owner joined. If it was not you, open People.',
        _ => 'A new phone or computer joined. If you did not invite it, open People.',
      },
    ),
    NoticeKind.clientRevoked => (
      title: 'Someone was removed from $feeder',
      body: 'A phone or computer can no longer reach it.',
    ),
    NoticeKind.unreachable => (
      title: "Can't reach $feeder",
      body: "This phone is home and hasn't reached it for a while. Check its power and your Wi-Fi.",
    ),
  };
}

/// How many portions of a failed feed went.
String _portions(NoticeDetail detail) => switch (detail) {
  NoticeDetail_Feed(:final requested, :final completed) when completed == 0 => 'None of $requested portions dropped.',
  NoticeDetail_Feed(:final requested, :final completed) => '$completed of $requested portions dropped.',
  _ => '',
};

/// Why a feed failed, briefly.
String _failedWhy(NoticeDetail detail) => switch (detail) {
  NoticeDetail_Feed(outcome: FeedOutcome.expired) => "It couldn't start in time.",
  NoticeDetail_Feed(reason: FeedReason.jammed || FeedReason.markStuck) => 'The wheel jammed.',
  NoticeDetail_Feed(reason: FeedReason.outletBlocked) => 'The outlet is blocked.',
  NoticeDetail_Feed(reason: FeedReason.power) => 'The power went out.',
  _ => 'The feeder stopped it.',
};

/// What a level is called in the settings.
String levelName(AlertLevel level) => switch (level) {
  AlertLevel.urgent => 'Urgent',
  AlertLevel.soon => 'Soon',
  AlertLevel.security => 'Security',
};

/// What a kind of notice is called in the settings.
String kindName(NoticeKind kind) => switch (kind) {
  NoticeKind.mealFailed => 'A meal failed',
  NoticeKind.mealUncertain => 'A meal may not have dropped',
  NoticeKind.mealMissed => 'A meal was missed',
  NoticeKind.mealNoRecord => 'A meal has no record',
  NoticeKind.powerWasOut => 'The power was out',
  NoticeKind.updateFailed => 'An update stopped',
  NoticeKind.rolledBack => 'Back on recovery firmware',
  NoticeKind.clientJoined => 'Someone joined',
  NoticeKind.clientRevoked => 'Someone was removed',
  NoticeKind.repeatedRestarts => 'Keeps restarting',
  NoticeKind.noClock => "Doesn't know the time",
  NoticeKind.cannotFeed => "Can't feed",
  NoticeKind.foodLow => 'Food is low',
  NoticeKind.foodLikelyOut => 'Probably out of food',
  NoticeKind.desiccantDue => 'Desiccant is due',
  NoticeKind.batteryBelowFloor => 'Batteries are low',
  NoticeKind.plansStuck => "Can't read its meals",
  NoticeKind.unreachable => "Can't reach it at home",
};

/// The words on the button that lets alerts through while notifications are
/// off, for what it does.
String notificationAskLabel(NotificationAsk ask) => switch (ask) {
  NotificationAsk.request => 'Allow notifications',
  NotificationAsk.openSettings => 'Open notification settings',
};
