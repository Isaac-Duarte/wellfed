import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/feeder_network.dart';
import 'package:wellfed/state/install/install_state.dart' show FeederJoinProblem;

void main() {
  const channel = MethodChannel('test/wifi');

  void answer(WidgetTester tester, Future<Object?> Function() reply) {
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (call) async {
      expect(call.method, 'joinFeeder');
      return reply();
    });
    addTearDown(() => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, null));
  }

  testWidgets('a join Android bound the process to goes on', (tester) async {
    answer(tester, () async => null);
    await joinByPicker(channel);
  });

  // Under a VPN the app can't bypass, Android joins the network but won't
  // bind the process to it, so the frame would leave on the home Wi-Fi.
  testWidgets('a process Android would not bind is refused as a VPN', (tester) async {
    answer(tester, () async => throw PlatformException(code: notBound));
    await expectLater(
      joinByPicker(channel),
      throwsA(isA<JoinRefused>().having((refused) => refused.problem, 'problem', FeederJoinProblem.vpn)),
    );
  });

  testWidgets('any other failure stays a failure, not a join', (tester) async {
    answer(tester, () async => throw PlatformException(code: 'unavailable'));
    await expectLater(joinByPicker(channel), throwsA(isA<PlatformException>()));
  });
}
