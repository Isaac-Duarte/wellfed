import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/home/feeder_card.dart' show skewWords;
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/model.dart' show Skew;
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

  testWidgets('a feeder of a newer revision opens as any other, and names this app to update', (tester) async {
    final book = FakeBook()..feeders = [feederKitchen];
    final ports = FakeFeederPorts();
    final fleet = FeederFleet(book, ports);
    addTearDown(fleet.dispose);
    await tester.pump();

    ports.session.opened(skew: Skew.feederAhead);
    await tester.pump();

    final FeederState? state = fleet[feederIdentity]?.state;
    expect(state?.connection, Connection.online);
    expect(state?.skew, Skew.feederAhead);
    expect(skewWords(state!), 'Update this app: the feeder runs newer firmware');
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
