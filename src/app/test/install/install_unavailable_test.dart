import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/state/install/install.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';
import 'install_support.dart';

void main() {
  test('a device that cannot join the feeder Wi-Fi is refused before anything is read', () async {
    final ports = FakeInstallPorts()..joinsFeederNetwork = false;
    final flow = InstallFlow(FakeBook(), ports);
    addTearDown(flow.dispose);

    await flow.load();

    expect(flow.state, isA<InstallUnavailable>());
    expect(flow.mayStart, isFalse);
    expect(ports.asked, 0);
    expect(ports.runs, isEmpty);
  });

  testWidgets('the screen says the computer cannot install, and offers no way on', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    try {
      final ports = FakeInstallPorts()..joinsFeederNetwork = false;
      await installOpen(tester, ports);

      expect(find.textContaining('This computer cannot install Wellfed.', findRichText: true), findsOneWidget);
      expect(find.text('Continue'), findsNothing);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });
}
