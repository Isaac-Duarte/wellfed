import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/wifi.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import '../support/fakes.dart';
import '../support/harness.dart';

void main() {
  group('Wi-Fi', () {
    Future<FakeSession> wifi(WidgetTester tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(fakeCard());
      await showTab(tester, WifiScreen(feeder: feeder));
      await tester.enterText(find.byType(TextField).first, 'Maple St 5G');
      await tester.enterText(find.byType(TextField).last, 'correcthorse');
      await settle(tester);
      return session;
    }

    testWidgets('saves once, then restarts to join', (tester) async {
      final FakeSession session = await wifi(tester);

      await tapIn(tester, find.text('Switch network'));
      expect(session.requests, hasLength(2));
      final put = session.requests.first as FeederRequest_PutWifi;
      expect(put.ssid, 'Maple St 5G');
      expect(put.passphrase, 'correcthorse');
      expect(session.requests.last, const FeederRequest.reboot());
    });

    testWidgets('what Rust or the feeder refuses restarts nothing', (tester) async {
      final FakeSession session = await wifi(tester);
      session.replies
        ..add(const FeederReply.notAsked(reason: NotAsked.credentials))
        ..add(const FeederReply.refused(refusal: FeederRefusal.forbidden()));

      await tapIn(tester, find.text('Switch network'));
      expect(find.textContaining('A network name is'), findsOneWidget);
      expect(session.requests, hasLength(1));

      await tapIn(tester, find.text('Switch network'));
      expect(find.textContaining("isn't allowed"), findsOneWidget);
      expect(session.requests, hasLength(2));
      expect(session.requests, isNot(contains(const FeederRequest.reboot())));
    });

    testWidgets('an unanswered save restarts nothing and is not sent again', (tester) async {
      final FakeSession session = await wifi(tester);
      session.replies.add(const FeederReply.unanswered());

      await tapIn(tester, find.text('Switch network'));
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(session.requests, hasLength(1));
    });

    testWidgets('forgetting asks first, goes once, and says if unanswered', (tester) async {
      final FakeSession session = await wifi(tester);
      session.replies.add(const FeederReply.unanswered());

      await tapIn(tester, find.text('Forget this network'));
      expect(session.requests, isEmpty);
      await tapIn(tester, find.widgetWithText(FilledButton, 'Forget'));

      expect(session.requests.single, isA<FeederRequest_ForgetWifi>());
      expect(find.textContaining('may or may not have'), findsOneWidget);
    });

    testWidgets('offers the networks this phone hears, and saves only a full password', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
      session.status(fakeCard());
      await showTab(tester, WifiScreen(feeder: feeder));
      bool saves() =>
          tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'Switch network')).onPressed != null;

      await tapIn(tester, find.text('Home'));
      await tapIn(tester, find.text('Neighbour 2.4'));
      await tester.enterText(find.byType(TextField), 'short');
      await settle(tester);
      expect(saves(), isFalse);

      await tester.enterText(find.byType(TextField), 'correcthorse');
      await settle(tester);
      await tapIn(tester, find.text('Switch network'));
      expect((session.requests.first as FeederRequest_PutWifi).ssid, 'Neighbour 2.4');
    });

    testWidgets('a restart that fails says the network is saved', (tester) async {
      final FakeSession session = await wifi(tester);
      session.replies
        ..add(const FeederReply.done(generation: 2))
        ..add(const FeederReply.refused(refusal: FeederRefusal.busy()));

      await tapIn(tester, find.text('Switch network'));
      expect(find.textContaining('The network is saved'), findsOneWidget);
    });
  });
}
