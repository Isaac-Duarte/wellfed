import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/home/set_aside.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/store_notice.dart';

import '../support/accessibility.dart';
import '../support/fakes.dart';

SetAside _file(StoreKind kind, String name) =>
    SetAside(kind: kind, movedTo: '/data/app/files/$name.damaged-1', reason: 'not this file', atMs: 1000);

Future<void> _show(WidgetTester tester, StoreNoticeFlow flow, {Brightness brightness = Brightness.light}) async {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    themed(
      ListenableBuilder(
        listenable: flow,
        builder: (context, _) => switch (flow.state) {
          final StoreNotice notice? => SetAsideScreen(notice: notice, onRead: flow.read),
          null => const Text('Home'),
        },
      ),
      brightness: brightness,
    ),
  );
  await settle(tester);
}

void main() {
  test('nothing set aside is no notice', () {
    final flow = StoreNoticeFlow(router: FakeRouter());
    addTearDown(flow.dispose);
    expect(flow.state, isNull);
  });

  testWidgets('a set-aside app file says what was kept and goes on to Home', (tester) async {
    final router = FakeRouter();
    final flow = StoreNoticeFlow(app: _file(StoreKind.app, 'wellfed.json'), router: router);
    addTearDown(flow.dispose);
    await _show(tester, flow);

    expect(find.textContaining("couldn't be read"), findsOneWidget);
    expect(find.textContaining('key and any saved copies of stock firmware'), findsOneWidget);
    expect(find.textContaining('Kept as wellfed.json.damaged-1'), findsOneWidget);
    expect(find.textContaining("Clear the app's data"), findsNothing);

    await tester.tap(find.text('Continue'));
    await settle(tester);
    expect(find.text('Home'), findsOneWidget);
  });

  testWidgets('a set-aside record of writes holds feeds until the owner checks the bowls', (tester) async {
    final router = FakeRouter()..setAsideFile = _file(StoreKind.pending, 'pending.json');
    final flow = StoreNoticeFlow(app: _file(StoreKind.app, 'wellfed.json'), router: router);
    addTearDown(flow.dispose);
    await _show(tester, flow);

    expect(find.textContaining('Feeding is paused.', findRichText: true), findsOneWidget);
    await tester.scrollUntilVisible(find.textContaining('Kept as pending.json.damaged-1'), 100);
    expect(find.textContaining('Kept as pending.json.damaged-1'), findsOneWidget);
    expect(find.text('Continue'), findsNothing);

    // Not written down: it stays, and feeds still wait.
    router.bowlsFail = true;
    await tester.tap(find.text("I've checked the bowls"));
    await settle(tester);
    expect(router.setAsideFile, isNotNull);
    expect(find.text("I've checked the bowls"), findsOneWidget);

    router.bowlsFail = false;
    await tester.tap(find.text("I've checked the bowls"));
    await settle(tester);
    expect(router.setAsideFile, isNull);
    expect(find.text('Home'), findsOneWidget);
  });

  for (final Brightness brightness in Brightness.values) {
    testWidgets('the notice is accessible in the ${brightness.name} theme', (tester) async {
      final router = FakeRouter()..setAsideFile = _file(StoreKind.pending, 'pending.json');
      final flow = StoreNoticeFlow(app: _file(StoreKind.app, 'wellfed.json'), router: router);
      addTearDown(flow.dispose);
      await _show(tester, flow, brightness: brightness);
      await expectAccessible(tester);
    });
  }
}
