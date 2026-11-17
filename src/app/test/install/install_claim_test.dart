import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/widgets/code_tiles.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';
import 'install_support.dart';

/// The claim follows the image the APK carries, as the run does: a consumer
/// image through Bluetooth setup, a bench image through its recovery window.
void main() {
  group('the claim route', () {
    // The route is the image's, as the run's is: a new Wi-Fi asked for on an
    // earlier stop puts no Bluetooth words on a bench image's claim.
    testWidgets('a bench image claims through its recovery window whatever an earlier stop asked', (tester) async {
      final ports = FakeInstallPorts(
        image: const ImageCheck.ready(provisioning: ImageProvisioning.bench, build: 'ab', version: 7),
      );
      await installStopped(tester, ports, const StopCase.notJoined(), resume: Resume.fromBank0);
      await installTap(tester, 'Enter the Wi-Fi again');
      await tester.enterText(find.byType(TextField).at(0), 'Maple St');
      await tester.enterText(find.byType(TextField).at(1), 'rightpassword');
      await tester.pump();
      await installTap(tester, 'Continue');

      expect(find.text('Wellfed is running. Now claim it.'), findsOneWidget);
      Finder bluetooth() => find.textContaining('Mic', findRichText: true);
      expect(bluetooth(), findsNothing);
      await tester.pump(InstallFlow.setupHintAfter);
      expect(bluetooth(), findsNothing);

      ports.run.ask(7, const InstallAsk.openRecoveryWindow());
      await settle(tester);
      expect(find.textContaining('then also the', findRichText: true), findsOneWidget);
      expect(bluetooth(), findsNothing);
    });

    testWidgets('a consumer image set up again shows the hold that starts setup, with its picture', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.notJoined(), resume: Resume.fromBank0);
      await installTap(tester, 'Enter the Wi-Fi again');
      await tester.enterText(find.byType(TextField).at(0), 'Maple St');
      await tester.enterText(find.byType(TextField).at(1), 'rightpassword');
      await tester.pump();
      await installTap(tester, 'Continue');

      expect(find.textContaining('Mic together', findRichText: true), findsOneWidget);
      expect(find.byType(PanelPhoto), findsOneWidget);
    });
    testWidgets('a broken-off claim names the route the image claims by', (tester) async {
      for (final (ImageProvisioning provisioning, String words, String not) in [
        (ImageProvisioning.consumer, 'The Bluetooth exchange broke off', 'recovery window'),
        (ImageProvisioning.bench, 'Open its recovery window again', 'Bluetooth'),
      ]) {
        final ports = FakeInstallPorts(
          image: ImageCheck.ready(provisioning: provisioning, build: 'ab', version: 7),
        );
        await installStopped(tester, ports, const StopCase.setupFailed(), resume: Resume.fromBank0);
        expect(find.textContaining(words, findRichText: true), findsOneWidget, reason: '$provisioning');
        expect(find.textContaining(not, findRichText: true), findsNothing, reason: '$provisioning');
        await tester.pumpWidget(const SizedBox());
      }
    });

    testWidgets('starting over lets the listener go before the Reset hold', (tester) async {
      final ports = FakeInstallPorts();
      await installStopped(tester, ports, const StopCase.uncertain());
      ports.listeners.single.updates.add(const ListenUpdate.stockStayed(resolved: true));
      await settle(tester);

      await installTap(tester, 'Start again');
      expect(ports.listeners.single.stops, 1);
    });
  });
}
