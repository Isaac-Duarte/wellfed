import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/state/fleet/one_at_a_time.dart';

void main() {
  test('a change waits for the one before it, even one that fails', () async {
    final queue = OneAtATime();
    final order = <String>[];
    final first = Completer<void>();

    final Future<void> saving = queue.run(() async {
      order.add('save starts');
      await first.future;
      order.add('save ends');
    });
    final Future<void> failing = queue.run(() async {
      order.add('second');
      throw StateError('refused');
    });
    final Future<void> clearing = queue.run(() async => order.add('clear'));

    await Future<void>.delayed(Duration.zero);
    expect(order, ['save starts'], reason: 'nothing overtakes a change still being written');
    first.complete();
    await saving;
    await expectLater(failing, throwsStateError);
    await clearing;
    expect(order, ['save starts', 'save ends', 'second', 'clear']);
  });
}
