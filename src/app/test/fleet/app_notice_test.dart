import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/app.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';

import '../support/fakes.dart';

/// A key the app holds but these tests never use.
class _Key with FakeOpaque implements ClientKey {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  testWidgets('a start whose record of writes was set aside opens on the bowls notice', (tester) async {
    tester.view.physicalSize = const Size(412, 915);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final router = FakeRouter()
      ..setAsideFile = const SetAside(
        kind: StoreKind.pending,
        movedTo: '/data/pending.json.damaged-1',
        reason: 'not this file',
        atMs: 1000,
      );
    final app = AppState.of(
      key: _Key(),
      store: const AppStore(label: 'Pixel', feeders: [], appearance: Appearance.system),
      path: '/data/wellfed.json',
      router: router,
    );

    await tester.pumpWidget(WellfedApp(app: app));
    await settle(tester);

    expect(find.textContaining('Feeding is paused.', findRichText: true), findsOneWidget);
    expect(find.text("I've checked the bowls"), findsOneWidget);
    expect(find.text('Wellfed'), findsNothing, reason: 'not the welcome');

    await tester.tap(find.text("I've checked the bowls"));
    await settle(tester);
    expect(router.setAsideFile, isNull);
    expect(find.text("I've checked the bowls"), findsNothing);
    await tester.pumpWidget(const SizedBox());
  });
}
