import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/widgets/feeder_drawing.dart';

import '../support/fakes.dart';

Future<void> _show(WidgetTester tester, {required double width, required VoidCallback onLock}) async {
  tester.view.physicalSize = Size(width, 600);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    themed(
      Scaffold(
        body: ListView(
          children: [
            FeederDrawing(
              height: 220,
              label: 'Drawing of the feeder',
              callouts: [
                const Callout(anchor: 'window', side: CalloutSide.right, lines: ['Food in']),
                Callout(
                  anchor: 'button_lock',
                  side: CalloutSide.left,
                  lines: const ['Buttons unlocked'],
                  onTap: onLock,
                  tapHint: 'Lock the buttons',
                ),
              ],
            ),
          ],
        ),
      ),
    ),
  );
  await settle(tester);
}

Finder get _picture => find.descendant(of: find.byType(FeederDrawing), matching: find.byType(CustomPaint)).first;

void main() {
  testWidgets('loads the traced lines and leads each callout out to its label', (tester) async {
    await _show(tester, width: 412, onLock: () {});

    expect(
      tester.renderObject(_picture),
      paints
        ..path()
        ..line()
        ..circle(),
    );
    expect(find.text('Food in'), findsOneWidget);
    expect(find.text('Buttons unlocked'), findsOneWidget);
  });

  testWidgets('reads out the picture and every callout, and the lock is a 48 dp button', (tester) async {
    final SemanticsHandle semantics = tester.ensureSemantics();
    var taps = 0;
    await _show(tester, width: 412, onLock: () => taps++);

    expect(find.bySemanticsLabel('Drawing of the feeder'), findsOneWidget);
    expect(find.bySemanticsLabel('Food in'), findsOneWidget);
    final SemanticsNode lock = tester.getSemantics(find.text('Buttons unlocked'));
    expect(lock, isSemantics(label: 'Buttons unlocked', isButton: true, hasTapAction: true));
    expect(lock.hintOverrides?.onTapHint, 'Lock the buttons');
    expect(lock.rect.height, greaterThanOrEqualTo(48));
    expect(lock.rect.width, greaterThanOrEqualTo(48));

    await tester.tap(find.text('Buttons unlocked'));
    expect(taps, 1);
    semantics.dispose();
  });

  testWidgets('too narrow for the labels, it lists them under the drawing', (tester) async {
    var taps = 0;
    await _show(tester, width: 240, onLock: () => taps++);

    expect(tester.renderObject(_picture), isNot(paints..line()));
    expect(find.text('Food in'), findsOneWidget);
    await tester.tap(find.text('Buttons unlocked'));
    expect(taps, 1);
    expect(
      tester.getSize(find.ancestor(of: find.text('Buttons unlocked'), matching: find.byType(InkWell))).height,
      greaterThanOrEqualTo(48),
    );
  });
}
