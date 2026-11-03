import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import '../support/fakes.dart';
import '../support/harness.dart';

/// What the feeder controller keeps of the portion and the meal call between
/// reads, and when it lets them go.
void main() {
  group('Kept readings', () {
    const calibration = CalibrationInfo(generation: 1, writable: true, portionMg: 12000, bowl: Bowl.single);
    const call = CallInfo(writable: true, clip: CallClip.none());

    Future<(FeederController, FakeSession)> kept(WidgetTester tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.replies
        ..add(const FeederReply.calibration(calibration: calibration))
        ..add(const FeederReply.call(call: call));
      await feeder.loadCalibration();
      await feeder.loadCall();
      expect(feeder.state.calibration, calibration);
      expect(feeder.state.call, call);
      return (feeder, session);
    }

    testWidgets('a portion write that ends any other way drops it until a read', (tester) async {
      final (FeederController feeder, FakeSession session) = await kept(tester);

      session.replies.add(const FeederReply.unanswered());
      await feeder.ask(
        const FeederRequest.putCalibration(base: 1, portion: PortionChange.milligrams(mg: 15000), bowl: Bowl.single),
      );
      expect(feeder.state.calibration, isNull);
      expect(feeder.state.call, call, reason: 'the clip is untouched');

      const weighed = CalibrationInfo(generation: 2, writable: true, portionMg: 15000, bowl: Bowl.single);
      session.replies.add(const FeederReply.calibration(calibration: weighed));
      await feeder.loadCalibration();
      expect(feeder.state.calibration, weighed);
    });

    for (final (String name, FeederRequest write) in [
      ('an upload', FeederRequest.uploadCall(wav: Uint8List.fromList(const [1, 2, 3]))),
      ('a delete', const FeederRequest.deleteCall()),
    ]) {
      testWidgets('$name that ends any other way drops the clip until a read', (tester) async {
        final (FeederController feeder, FakeSession session) = await kept(tester);

        session.replies.add(const FeederReply.done());
        await feeder.ask(write);
        expect(feeder.state.call, isNull);
        expect(feeder.state.calibration, calibration, reason: 'the portion is untouched');

        const stored = CallInfo(writable: true, clip: CallClip.stored(bytes: 16800, crc: 1), ms: 4200);
        session.replies.add(const FeederReply.call(call: stored));
        await feeder.loadCall();
        expect(feeder.state.call, stored);
      });
    }

    testWidgets('a session opened again drops both', (tester) async {
      final (FeederController feeder, FakeSession session) = await kept(tester);

      session.opened();
      await tester.pump();

      expect(feeder.state.calibration, isNull);
      expect(feeder.state.call, isNull);
    });
  });
}
