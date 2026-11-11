import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/run_holds.dart';
import 'package:wellfed/src/rust/api/alerts.dart' show AlertHold;

void main() {
  late List<String?> applied;
  late Map<AlertHold, bool> marked;
  late RunHolds holds;

  setUp(() {
    applied = [];
    marked = {};
    holds = RunHolds((text) async {
      applied.add(text);
      return true;
    }, (run, {required held}) => marked[run] = held);
  });

  test('a run that ends never lets go of another run still holding', () async {
    await holds.hold(AlertHold.install, 'Installing on Kitchen');
    await holds.hold(AlertHold.restore, 'Restoring Hall');
    await holds.release(AlertHold.install);

    expect(holds.held, {AlertHold.restore});
    expect(applied.last, 'Restoring Hall', reason: 'the process stays held, in the words of the restore');
    expect(marked, {AlertHold.install: false, AlertHold.restore: true});

    await holds.release(AlertHold.restore);
    expect(holds.held, isEmpty);
    expect(applied.last, isNull, reason: 'the last run lets the process go');
  });

  test('releasing a run that does not hold changes nothing', () async {
    await holds.hold(AlertHold.update, 'Updating Kitchen');
    await holds.release(AlertHold.install);
    expect(holds.held, {AlertHold.update});
    expect(applied, ['Updating Kitchen']);
  });

  test('a run holding again takes the newest words', () async {
    await holds.hold(AlertHold.install, 'Installing');
    await holds.hold(AlertHold.update, 'Updating');
    await holds.hold(AlertHold.install, 'Installing again');
    await holds.release(AlertHold.install);
    expect(applied.last, 'Updating');
  });
}
