import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/platform/notices.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/alerts/alert_words.dart';

void main() {
  const NoticeWords words = (title: 'Kitchen: a meal failed', body: 'The wheel jammed.');

  test('the same words again do not alert, and new words do', () {
    const showing = [ActiveNotification(id: 7, title: 'Kitchen: a meal failed', bigText: 'The wheel jammed.')];
    expect(showsWords(showing, 7, words), isTrue);
    expect(showsWords(showing, 8, words), isFalse, reason: 'another notification');
    const uncertain = [ActiveNotification(id: 7, title: 'Kitchen: a meal may have run', bigText: 'Unsure.')];
    expect(showsWords(uncertain, 7, words), isFalse, reason: 'an uncertain meal found failed alerts again');
    expect(showsWords(const [], 7, words), isFalse, reason: 'swiped away, or never shown');
  });

  test('the button asks Android until a request was refused, and below 13 opens the settings', () {
    expect(notificationAskFor(sdkAtLeast33: true, refusedBefore: false), NotificationAsk.request);
    expect(notificationAskFor(sdkAtLeast33: true, refusedBefore: true), NotificationAsk.openSettings);
    expect(notificationAskFor(sdkAtLeast33: false, refusedBefore: false), NotificationAsk.openSettings);
    expect(notificationAskFor(sdkAtLeast33: false, refusedBefore: true), NotificationAsk.openSettings);
  });

  group('a round', () {
    const feeders = [KnownFeeder(identity: 'aa', name: 'Kitchen'), KnownFeeder(identity: 'bb', name: 'Hall')];

    Future<void> round(RoundFailures failures, List<String> reached) => feederRound(feeders, failures, (feeder) async {
      if (feeder.identity == 'aa') throw StateError('the verifier was never started');
      reached.add(feeder.identity);
    });

    test('in the background, a feeder whose catch-up throws leaves the next its turn', () async {
      final reached = <String>[];
      await round(RoundFailures.logged, reached);
      expect(reached, ['bb']);
    });

    test('asked for, it gives every feeder its turn, then throws the failure', () async {
      final reached = <String>[];
      await expectLater(round(RoundFailures.thrown, reached), throwsStateError);
      expect(reached, ['bb'], reason: 'the round finished first');
    });
  });

  test('a sweep takes only alert notifications', () {
    const active = [
      ActiveNotification(id: 7, channelId: 'keep-alive'),
      ActiveNotification(id: 0x40000001, channelId: 'alerts-urgent'),
      ActiveNotification(id: 0x40000002, channelId: 'alerts-security'),
      ActiveNotification(channelId: 'alerts-soon'),
    ];
    expect(alertNotificationIds(active), [0x40000001, 0x40000002]);
  });
}
