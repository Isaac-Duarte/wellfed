import 'package:clock/clock.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'feeder_support.dart';

void main() {
  testWidgets('offline, Today says when it last heard the feeder, and claims nothing is fine now', (tester) async {
    final heard = DateTime(2026, 11, 2, 14, 58);
    await withClock(Clock.fixed(heard), () async {
      final (FeederFleet fleet, FakeSession session) = await feederFleet(tester);
      await feederShowShell(tester, fleet);
      session.status(fullCard());
      await settle(tester);
      expect(find.text('Power, Wi-Fi and clock look fine'), findsOneWidget);
      expect(find.textContaining('Last heard'), findsNothing);

      session.updates.add(
        const SessionUpdate.closed(
          error: BridgeError(kind: ErrorKind.closed, message: 'gone'),
        ),
      );
      await session.updates.close();
      await tester.pump();

      expect(find.text("Can't reach it"), findsOneWidget);
      expect(find.text('Last heard 2:58 PM'), findsOneWidget);
      expect(find.text('Power, Wi-Fi and clock look fine'), findsNothing);
      expect(find.text('Power, Wi-Fi and clock were fine then'), findsOneWidget);
      // Let the next try start, so no backoff is left pending at the end.
      await tester.pump(const Duration(minutes: 1));
    });
  });
}
