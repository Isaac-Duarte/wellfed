import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'feeder_support.dart';

void main() {
  Future<(FakeBook, FeederController)> shell(WidgetTester tester) async {
    final book = FakeBook()..feeders = [feederKitchen];
    final fleet = FeederFleet(book, FakeFeederPorts());
    addTearDown(fleet.dispose);
    await tester.pump();
    final FeederController feeder = feederOnly(fleet);
    await showTab(tester, FeederShell(feeder: feeder));
    return (book, feeder);
  }

  Future<void> rename(WidgetTester tester, String name) async {
    await tapIn(tester, find.text('Settings'));
    await tapIn(tester, find.text('Name'));
    await tester.enterText(find.byType(TextField), name);
    await tapIn(tester, find.widgetWithText(FilledButton, 'Rename'));
  }

  testWidgets('a new name is kept on this phone, and the title follows it', (tester) async {
    final (FakeBook book, _) = await shell(tester);

    await rename(tester, '  Pantry  ');
    expect(book.feeders.single.name, 'Pantry');
    expect(book.feeders.single.identity, feederKitchen.identity);
    expect(find.widgetWithText(AppBar, 'Pantry'), findsOneWidget);
  });

  testWidgets('a blank name changes nothing', (tester) async {
    final (FakeBook book, _) = await shell(tester);

    await rename(tester, '   ');
    expect(book.feeders.single.name, feederKitchen.name);
    expect(find.widgetWithText(AppBar, feederKitchen.name), findsOneWidget);
  });

  testWidgets('a name that cannot be saved says so and stays as it was', (tester) async {
    final (FakeBook book, _) = await shell(tester);
    book.renameFails = const BridgeError(kind: ErrorKind.store, message: 'read-only');

    await rename(tester, 'Pantry');
    expect(find.text("Couldn't save the name. Try again."), findsOneWidget);
    expect(find.widgetWithText(AppBar, feederKitchen.name), findsOneWidget);
  });
}
