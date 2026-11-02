import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/fleet/fleet.dart';

import '../support/fakes.dart';
import 'feeder_support.dart';

void main() {
  Future<void> drop(WidgetTester tester, FakeSession session) async {
    session.updates.add(
      const SessionUpdate.closed(
        error: BridgeError(kind: ErrorKind.closed, message: 'gone'),
      ),
    );
    await session.updates.close();
    await tester.pump();
  }

  testWidgets('coming back to the app reconnects at once, not after the backoff', (tester) async {
    final book = FakeBook()..feeders = [feederKitchen];
    final ports = FakeFeederPorts();
    final fleet = FeederFleet(book, ports);
    addTearDown(fleet.dispose);
    await tester.pump();
    ports.session.opened();
    await tester.pump();

    // The link drops in the background, and the first try again fails, so the
    // next waits 6 s.
    await drop(tester, ports.session);
    await tester.pump(const Duration(seconds: 3));
    expect(ports.sessions, hasLength(2));
    await drop(tester, ports.session);
    await tester.pump(const Duration(seconds: 1));
    expect(ports.sessions, hasLength(2));

    fleet.resumed();
    await tester.pump();
    expect(ports.sessions, hasLength(3));
  });
}
