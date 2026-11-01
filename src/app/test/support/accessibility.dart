import 'package:flutter_test/flutter_test.dart';

/// What every screen owes a person with a screen reader, a large finger or low
/// vision: Android's 48dp targets, a label on each target, and contrast.
Future<void> expectAccessible(WidgetTester tester) async {
  final SemanticsHandle handle = tester.ensureSemantics();
  await tester.pump();
  await expectLater(tester, meetsGuideline(androidTapTargetGuideline));
  await expectLater(tester, meetsGuideline(labeledTapTargetGuideline));
  await expectLater(tester, meetsGuideline(textContrastGuideline));
  handle.dispose();
}
