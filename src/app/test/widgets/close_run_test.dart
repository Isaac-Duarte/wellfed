import 'dart:async';
import 'dart:ui' show AppExitResponse;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/alerts.dart' show AlertHold;
import 'package:wellfed/widgets/close_run.dart';

import '../support/fakes.dart';

void main() {
  Future<bool?> ask(WidgetTester tester, Set<AlertHold> runs, String button) async {
    bool? answer;
    await tester.pumpWidget(
      themed(
        Builder(
          builder: (context) => TextButton(
            onPressed: () async => answer = await showDialog<bool>(
              context: context,
              builder: (_) => CloseRunDialog(runs: runs),
            ),
            child: const Text('Close'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Close'));
    await tester.pumpAndSettle();
    expect(find.text('Close Wellfed?'), findsOneWidget);
    expect(find.textContaining('may leave the feeder mid-step'), findsOneWidget);
    await tester.tap(find.text(button));
    await tester.pumpAndSettle();
    return answer;
  }

  testWidgets('closing mid-install asks, and keeping on keeps the window', (tester) async {
    expect(await ask(tester, {AlertHold.install}, 'Keep going'), isFalse);
  });

  testWidgets('the owner can close anyway', (tester) async {
    expect(await ask(tester, {AlertHold.update}, 'Close anyway'), isTrue);
  });

  for (final (AlertHold run, String what) in [
    (AlertHold.install, 'An install'),
    (AlertHold.update, 'A firmware update'),
    (AlertHold.restore, 'A restore of the original firmware'),
  ]) {
    testWidgets('it names ${run.name}, the newest run', (tester) async {
      await tester.pumpWidget(themed(CloseRunDialog(runs: {AlertHold.install, run})));
      expect(find.textContaining('$what is still going'), findsOneWidget);
    });
  }

  group('The exit request', () {
    late Set<AlertHold> going;
    late BuildContext context;
    late CloseRunGuard guard;

    Future<void> pump(WidgetTester tester) async {
      going = {};
      guard = CloseRunGuard(runs: () => going, context: () => context);
      await tester.pumpWidget(
        themed(
          Builder(
            builder: (at) {
              context = at;
              return const SizedBox();
            },
          ),
        ),
      );
    }

    testWidgets('passes through with no run going, asking nothing', (tester) async {
      await pump(tester);
      expect(await guard.exitRequested(), AppExitResponse.exit);
      expect(find.byType(CloseRunDialog), findsNothing);
    });

    for (final (String button, AppExitResponse response) in [
      ('Keep going', AppExitResponse.cancel),
      ('Close anyway', AppExitResponse.exit),
    ]) {
      testWidgets('mid-run asks, and "$button" answers ${response.name}', (tester) async {
        await pump(tester);
        going = {AlertHold.install};
        AppExitResponse? answer;
        unawaited(guard.exitRequested().then((given) => answer = given));
        await tester.pumpAndSettle();
        expect(find.byType(CloseRunDialog), findsOneWidget);

        await tester.tap(find.text(button));
        await tester.pumpAndSettle();
        expect(answer, response);
      });
    }

    testWidgets('a second close while it asks opens no second dialog', (tester) async {
      await pump(tester);
      going = {AlertHold.update};
      AppExitResponse? first;
      unawaited(guard.exitRequested().then((given) => first = given));
      await tester.pumpAndSettle();

      expect(await guard.exitRequested(), AppExitResponse.cancel);
      await tester.pumpAndSettle();
      expect(find.byType(CloseRunDialog), findsOneWidget);

      await tester.tap(find.text('Close anyway'));
      await tester.pumpAndSettle();
      expect(first, AppExitResponse.exit);
    });
  });
}
