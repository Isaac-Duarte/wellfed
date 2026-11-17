import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// Where the background engine starts Dart, as BackgroundEngine.kt names it.
const _engine = 'android/app/src/main/kotlin/io/github/isaacduarte/wellfed/BackgroundEngine.kt';

/// The `package:wellfed/` libraries `lib/main.dart` imports or exports, at
/// any depth: what the compiled program holds of the app's own code.
Set<String> _reachedFromMain() {
  final directive = RegExp(r'''^\s*(?:import|export)\s+'package:wellfed/([^']+)'.*;''', multiLine: true);
  final reached = <String>{};
  final pending = ['main.dart'];
  while (pending.isNotEmpty) {
    final String path = pending.removeLast();
    if (!reached.add(path)) continue;
    final file = File('lib/$path');
    if (!file.existsSync()) continue;
    for (final RegExpMatch match in directive.allMatches(file.readAsStringSync())) {
      pending.add(match.group(1) ?? '');
    }
  }
  return reached;
}

String _constant(String source, String name) {
  final RegExpMatch? match = RegExp('const val $name = "([^"]+)"').firstMatch(source);
  expect(match, isNotNull, reason: '$_engine no longer names $name');
  return match?.group(1) ?? '';
}

void main() {
  // Android starts the background engine at a library and function by name.
  // A library nothing in the program imports isn't compiled into the release
  // build, and the engine then fails to start with no error anywhere a test
  // or the owner would see: no catch-up or alert runs while the app is closed.
  test("the background engine's entry point is compiled into the app", () {
    final String engine = File(_engine).readAsStringSync();
    final String library = _constant(engine, 'ENTRY_LIBRARY');
    final String entry = _constant(engine, 'ENTRY');

    expect(library, startsWith('package:wellfed/'));
    final String path = library.substring('package:wellfed/'.length);
    expect(_reachedFromMain(), contains(path), reason: 'lib/main.dart must import or export $library');

    // Kept by name even where nothing calls it.
    final String source = File('lib/$path').readAsStringSync();
    expect(
      RegExp("@pragma\\('vm:entry-point'\\)\\s+(?:Future<void>|void)\\s+$entry\\(").hasMatch(source),
      isTrue,
      reason: "$library must declare $entry() under @pragma('vm:entry-point')",
    );
  });
}
