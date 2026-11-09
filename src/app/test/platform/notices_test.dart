import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/platform/notices.dart';
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

  test("a refused ask opens Android's settings only for a lasting denial", () {
    bool opens(bool? before, bool? after, {bool silentBefore = false}) =>
        opensNotificationSettings(before: before, after: after, silentBefore: silentBefore);

    expect(opens(false, true), isFalse, reason: 'the first denial, in the dialog');
    expect(opens(true, true), isFalse);
    expect(opens(true, false), isTrue, reason: 'a lasting denial, just made');
    expect(opens(false, false), isFalse, reason: 'the dialog dismissed, or a denial from before');
    expect(opens(false, false, silentBefore: true), isTrue, reason: 'a second tap with no dialog');
    expect(opens(null, null), isTrue, reason: 'below Android 13 only the settings turn them on');
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
