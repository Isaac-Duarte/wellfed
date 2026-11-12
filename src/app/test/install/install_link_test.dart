import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/install.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';
import 'install_support.dart';

/// The claim's Bluetooth link: a feeder still linked does not advertise, so
/// every link the flow opened is hung up, whenever the run or the screen ends.
void main() {
  for (final (how, end) in [
    ('the run ends', (WidgetTester tester, FakeRun run) async => run.stopped(const StopCase.cancelled())),
    ('the screen goes', (WidgetTester tester, FakeRun run) => tester.pumpWidget(const SizedBox())),
  ]) {
    testWidgets('a link that opens after $how is hung up, so the feeder advertises again', (tester) async {
      final ports = FakeInstallPorts()..linkGate = Completer<void>();
      final FakeRun run = await installAtClaim(tester, ports);
      run.ask(3, const InstallAsk.openLink());
      await settle(tester);

      await end(tester, run);
      await settle(tester);
      ports.linkGate?.complete();
      await tester.pump();
      expect(ports.link.closes, 1);
      expect(run.answers[3], isNull, reason: 'nobody is left to answer');
      await tester.pumpWidget(const SizedBox());
    });
  }

  testWidgets('a link write with no link open fails, not done', (tester) async {
    final ports = FakeInstallPorts();
    final FakeRun run = await installAtClaim(tester, ports);
    run.ask(
      4,
      InstallAsk.writeLink(
        pieces: [
          Uint8List.fromList(const [1, 2]),
        ],
      ),
    );
    await settle(tester);
    expect(run.answers[4], isA<InstallAnswer_Failed>());
    expect(ports.link.written, isEmpty);
    await tester.pumpWidget(const SizedBox());
  });
}
