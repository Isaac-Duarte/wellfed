import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/install_ports.dart';
import 'package:wellfed/state/settings/backups.dart';

import '../support/fakes.dart';

/// Ports whose backup is handed on by `save`, as a computer's are.
class _SavingPorts implements BackupPorts {
  _SavingPorts(this.save);

  final SaveBackup save;

  @override
  Future<String?> kept(String identity) async => null;

  @override
  Future<void> share(String path) => save(path);
}

void main() {
  late Directory dir;
  late File backup;

  setUp(() {
    dir = Directory.systemTemp.createTempSync('wellfed-save-backup');
    backup = File('${dir.path}/stock-1.1.15.bin')..writeAsStringSync('stock');
  });
  tearDown(() => dir.deleteSync(recursive: true));

  test('the dialog is offered the backup by its own name, and a copy goes where it says', () async {
    final asked = <String>[];
    final save = SaveBackup(
      pick: (name) async {
        asked.add(name);
        return '${dir.path}/kept.bin';
      },
    );

    await save(backup.path);
    expect(asked, ['stock-1.1.15.bin']);
    expect(File('${dir.path}/kept.bin').readAsStringSync(), 'stock');
  });

  test('cancelling the dialog saves nothing and says nothing', () async {
    final flow = BackupsFlow(FakeBook(), _SavingPorts(SaveBackup(pick: (_) async => null)));
    addTearDown(flow.dispose);

    expect(await flow.share(backup.path), isNull);
    expect(dir.listSync(), hasLength(1));
  });

  test('a copy that fails is reported, not lost', () async {
    final flow = BackupsFlow(FakeBook(), _SavingPorts(SaveBackup(pick: (_) async => '${dir.path}/missing/kept.bin')));
    addTearDown(flow.dispose);

    expect(await flow.share(backup.path), startsWith("Couldn't save the backup:"));
  });
}
