import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/setup/pairing_view.dart';
import 'package:wellfed/src/rust/api/setup.dart';
import 'package:wellfed/state/pairing/pairing.dart';

import '../support/fakes.dart';

/// A feeder that proved the code and had the last step goes on without the
/// phone, so a link that drops then says nothing of the outcome: the phone
/// saves the feeder first, and a session on the LAN settles whether it took.
void main() {
  final proven = Uint8List.fromList(const [0xab, 0xcd]);

  /// A BLE setup whose link drops once the last step went.
  Future<PairingFlow> dropped(FakeBook book, FakePairingPorts ports) async {
    final flow = PairingFlow(book, ports);
    addTearDown(flow.dispose);
    unawaited(
      flow.runSetup(
        wifi: const WifiCredentials(ssid: 'Maple St', passphrase: 'x'),
        name: 'Kitchen',
        device: feederNearby('feeder-1'),
      ),
    );
    await pumpEventQueue();
    ports.drivers.single.provenFeeder = proven;
    await ports.link.bytes.close();
    await pumpEventQueue();
    return flow;
  }

  test('a session that opens says it took, and the feeder is saved where it answers', () async {
    final book = FakeBook();
    final ports = FakePairingPorts();
    final PairingFlow flow = await dropped(book, ports);

    expect(ports.settled, [('abcd', '192.168.1.40:4747')]);
    expect(flow.state, isA<PairingDone>().having((done) => done.enrolled, 'enrolled', isNull));
    expect(book.feeders.single.address, '192.168.1.40:4747');
  });

  test('a session the feeder refuses says it never took, and forgets the feeder', () async {
    final book = FakeBook();
    final ports = FakePairingPorts()..settles = SetupSettled.notEnrolled;
    final PairingFlow flow = await dropped(book, ports);

    expect(flow.state, isA<PairingFailed>().having((failed) => failed.failure, 'failure', PairingFailure.notTaken));
    expect(book.feeders, isEmpty);
  });

  testWidgets('a feeder not found keeps it saved and says the outcome is unknown', (tester) async {
    tester.view.physicalSize = const Size(1080, 3000);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    final book = FakeBook();
    final ports = FakePairingPorts()..located = null;
    late PairingFlow flow;
    await tester.runAsync(() async => flow = await dropped(book, ports));
    await tester.pumpWidget(themed(PairingView(flow: flow, ble: true, onRetry: () {}, onNewCode: () {})));
    await settle(tester);

    expect(flow.state, isA<PairingFailed>().having((failed) => failed.failure, 'failure', PairingFailure.unsettled));
    expect(book.feeders.single.identity, 'abcd');
    expect(find.text('Setup may or may not have finished'), findsOneWidget);
    expect(find.text('Go to Home'), findsOneWidget);
    expect(find.textContaining('stays in setup mode'), findsNothing);
  });

  test('a recovery whose connection went after the last step is settled where it was', () async {
    final book = FakeBook();
    final ports = FakePairingPorts();
    final flow = PairingFlow(book, ports);
    addTearDown(flow.dispose);
    unawaited(flow.runRecovery(address: '192.168.1.41:4747', name: 'Kitchen'));
    await pumpEventQueue();

    final FakeRecovery recovery = ports.recoveries.single;
    recovery.steps.add(SetupStep.unsettled(feeder: proven));
    await recovery.steps.close();
    await pumpEventQueue();

    expect(ports.settled, [('abcd', '192.168.1.41:4747')]);
    expect(flow.state, isA<PairingDone>());
  });

  test('a paired feeder is saved before the phone looks for it on the LAN', () async {
    final book = FakeBook();
    final ports = FakePairingPorts()..locateGate = Completer<void>();
    final flow = PairingFlow(book, ports);
    addTearDown(flow.dispose);
    unawaited(
      flow.runSetup(
        wifi: const WifiCredentials(ssid: 'Maple St', passphrase: 'x'),
        name: 'Kitchen',
        device: feederNearby('feeder-1'),
      ),
    );
    await pumpEventQueue();
    ports.drivers.single.script = [doneStep];
    ports.link.bytes.add(const [1]);
    await pumpEventQueue();

    expect(flow.state, isA<PairingJoining>());
    expect(book.feeders.single.identity, 'abcd', reason: 'saved while it looks');
    expect(book.feeders.single.address, isNull);
  });

  test('a recovery that ends as its screen closes still saves the feeder', () async {
    final book = FakeBook();
    final ports = FakePairingPorts();
    final flow = PairingFlow(book, ports);
    unawaited(flow.runRecovery(address: '192.168.1.41:4747', name: 'Kitchen'));
    await pumpEventQueue();

    final FakeRecovery recovery = ports.recoveries.single;
    recovery.steps.add(doneStep);
    await pumpEventQueue();
    flow.dispose();
    await recovery.steps.close();
    await pumpEventQueue();

    expect(book.feeders.single.identity, 'abcd');
  });
}
