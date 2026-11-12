part of 'feed_sheet.dart';

/// Why a feed's end is not known, in the sheet's words.
String _uncertainWhy(FeedReason reason) => switch (reason) {
  FeedReason.interrupted =>
    "The feeder restarted during the feed, so it can't say how much "
        'dropped.',
  FeedReason.power =>
    "The power went out during the feed, so the feeder can't say how much "
        'dropped.',
  FeedReason.sensor ||
  FeedReason.noFeedback ||
  FeedReason.feedbackStream => "The feeder lost track of its wheel, so it can't say how much dropped.",
  _ => 'The feeder stopped without recording how much dropped.',
};

/// Why the feeder refused a feed or its stop, in the sheet's words.
String _refusal(FeedRefusal? refusal) => switch (refusal) {
  FeedRefusal.forbidden => 'This $deviceName may not feed this feeder.',
  FeedRefusal.busy => "It's busy saving a change. Try again in a moment.",
  FeedRefusal.fault => 'It stopped on a fault and feeds again after a restart.',
  FeedRefusal.portions => 'A feed is ${Limits.portionsLeast} to ${Limits.portionsMost} portions.',
  FeedRefusal.notFound => 'That feed is not running.',
  FeedRefusal.unavailable => 'This firmware does not feed.',
  FeedRefusal.other || null => 'It refused the request.',
};
