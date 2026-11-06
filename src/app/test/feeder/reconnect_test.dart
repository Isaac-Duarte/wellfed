import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
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

  testWidgets('a feeder of another build reads as needing an update, and backs off', (tester) async {
    final book = FakeBook()..feeders = [feederKitchen];
    final ports = FakeFeederPorts();
    final fleet = FeederFleet(book, ports);
    addTearDown(fleet.dispose);
    await tester.pump();

    // Each session opens, then takes nothing but an update: its first read
    // ends it.
    Future<void> otherBuild() async {
      ports.session.opened();
      await tester.pump();
      ports.session.updates.add(
        const SessionUpdate.closed(
          error: BridgeError(kind: ErrorKind.otherBuild, message: 'a different build'),
        ),
      );
      await ports.session.updates.close();
      await tester.pump();
    }

    await otherBuild();
    expect(fleet[feederIdentity]?.state.connection, Connection.otherBuild);
    expect(fleet[feederIdentity]?.state.canFeed, isFalse);

    // Opened all the same, the wait still doubles: 3 s, then 6 s.
    await tester.pump(const Duration(seconds: 3));
    expect(ports.sessions, hasLength(2));
    await otherBuild();
    await tester.pump(const Duration(seconds: 3));
    expect(ports.sessions, hasLength(2));
    await tester.pump(const Duration(seconds: 3));
    expect(ports.sessions, hasLength(3));
  });

  testWidgets('a feeder of another major reads as needing another app, until a new try clears it', (tester) async {
    final book = FakeBook()..feeders = [feederKitchen];
    final ports = FakeFeederPorts();
    final fleet = FeederFleet(book, ports);
    addTearDown(fleet.dispose);
    await tester.pump();

    // The handshake refuses the session: it never opens.
    ports.session.updates.add(
      const SessionUpdate.closed(
        error: BridgeError(kind: ErrorKind.version, message: 'major 2'),
      ),
    );
    await ports.session.updates.close();
    await tester.pump();
    expect(fleet[feederIdentity]?.state.connection, Connection.otherVersion);
    expect(fleet[feederIdentity]?.state.error, ErrorKind.version);

    // The next try starts from no error: the last one no longer says how it
    // stands.
    await tester.pump(const Duration(seconds: 3));
    expect(ports.sessions, hasLength(2));
    expect(fleet[feederIdentity]?.state.connection, Connection.connecting);
    expect(fleet[feederIdentity]?.state.error, isNull);
  });
}
