import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/state/wifi/networks.dart';
import 'package:wellfed/widgets/network_picker.dart';

import '../support/fakes.dart';

/// Home on both bands, a café nearby, a 5 GHz-only network and an open one.
const List<SeenNetwork> _heard = [
  SeenNetwork(ssid: 'Home', frequencyMhz: 2437, level: -70, secured: true),
  SeenNetwork(ssid: 'Home', frequencyMhz: 5180, level: -40, secured: true),
  SeenNetwork(ssid: 'Home', frequencyMhz: 2412, level: -66, secured: true),
  SeenNetwork(ssid: 'Cafe', frequencyMhz: 2462, level: -50, secured: true),
  SeenNetwork(ssid: 'Fast', frequencyMhz: 5745, level: -30, secured: true),
  SeenNetwork(ssid: 'Library', frequencyMhz: 2412, level: -58, secured: false),
  SeenNetwork(ssid: '', frequencyMhz: 2412, level: -20, secured: true),
];

Future<TextEditingController> _show(WidgetTester tester, NetworkScan scan, {String? current}) async {
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final controller = TextEditingController();
  addTearDown(controller.dispose);
  await tester.pumpWidget(
    themed(
      Scaffold(
        body: Padding(
          padding: const EdgeInsets.all(16),
          child: NetworkPicker(controller: controller, scan: () async => scan, current: () async => current),
        ),
      ),
    ),
  );
  await settle(tester);
  return controller;
}

void main() {
  group('the choices', () {
    test('merge each name, keep its strongest 2.4 GHz radio, and drop hidden names', () {
      final List<NetworkChoice> choices = networkChoices(_heard);

      expect(choices.map((choice) => choice.ssid), ['Cafe', 'Library', 'Home', 'Fast']);
      final NetworkChoice home = choices.firstWhere((choice) => choice.ssid == 'Home');
      expect(home.level, -66, reason: 'its 5 GHz radio is stronger, but the feeder never uses it');
      expect(home.joinable, isTrue);
    });

    test("this phone's network leads, and a 5 GHz-only one trails", () {
      final List<NetworkChoice> choices = networkChoices(_heard, current: '"Home"');

      expect(choices.first.ssid, 'Home');
      expect(choices.first.current, isTrue);
      expect(choices.last.ssid, 'Fast');
      expect(choices.last.joinable, isFalse);
    });

    test("this phone's network leads even when the scan missed it", () {
      final List<NetworkChoice> choices = networkChoices(_heard, current: 'Attic');

      expect(choices.first.ssid, 'Attic');
      expect(choices.first.level, isNull);
      expect(choices, hasLength(5));
    });
  });

  group('the picker', () {
    testWidgets("chooses this phone's network first, and lists the rest by signal", (tester) async {
      final TextEditingController picked = await _show(tester, const NetworksSeen(_heard), current: 'Home');
      expect(picked.text, 'Home');

      await tester.tap(find.text('Home'));
      await settle(tester);
      expect(find.text("This phone's network · Fair"), findsOneWidget);
      expect(find.text('No password · Strong'), findsOneWidget);
      expect(find.text("5 GHz only: the feeder can't join"), findsOneWidget);
      final List<double> rows = [
        for (final name in ['Cafe', 'Library', 'Fast']) tester.getTopLeft(find.text(name)).dy,
      ];
      expect(rows, orderedEquals([...rows]..sort()));

      await tester.tap(find.text('Cafe'));
      await settle(tester);
      expect(picked.text, 'Cafe');
      expect(find.text('Choose a network'), findsNothing);
    });

    testWidgets('a 5 GHz-only network cannot be picked', (tester) async {
      final TextEditingController picked = await _show(tester, const NetworksSeen(_heard), current: 'Home');

      await tester.tap(find.text('Home'));
      await settle(tester);
      await tester.tap(find.text('Fast'));
      await settle(tester);
      expect(picked.text, 'Home');
      expect(find.text('Scan again'), findsOneWidget, reason: 'the sheet stays open');
    });

    testWidgets('a hidden network is typed', (tester) async {
      final TextEditingController picked = await _show(tester, const NetworksSeen(_heard), current: 'Home');

      await tester.tap(find.text('Home'));
      await settle(tester);
      await tester.tap(find.text('Type a network name'));
      await settle(tester);
      expect(picked.text, isEmpty);

      await tester.enterText(find.byType(TextField), 'Attic');
      expect(picked.text, 'Attic');
      await tester.tap(find.text('Choose from the list'));
      await settle(tester);
      expect(find.byType(TextField), findsNothing);
    });

    testWidgets('a phone that lists nothing says why and takes a typed name', (tester) async {
      final TextEditingController picked = await _show(tester, const NetworksUnread(ScanProblem.permission));

      expect(find.textContaining('Allow location'), findsOneWidget);
      await tester.enterText(find.byType(TextField), 'Home');
      expect(picked.text, 'Home');
      expect(find.text('Choose from the list'), findsNothing);
    });
  });
}
