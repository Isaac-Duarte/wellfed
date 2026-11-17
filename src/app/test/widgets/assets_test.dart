import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// Every asset a screen names by a literal path is in the bundle: a picture
/// whose file or pubspec entry went missing shows as an empty box on the
/// phone, and no golden of a screen that never shows it would catch that.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('every asset path the app names loads from the bundle', () async {
    final literal = RegExp(r"""['"](assets/[^'"$]+\.(?:png|jpg|svg|json))['"]""");
    final named = <String>{
      for (final file in Directory('lib').listSync(recursive: true).whereType<File>())
        if (file.path.endsWith('.dart'))
          for (final match in literal.allMatches(file.readAsStringSync()))
            if (match.group(1) case final path?) path,
    };

    expect(named, contains('assets/renders/panel.png'), reason: 'the scan found the panel picture');
    for (final path in named) {
      final ByteData data = await rootBundle.load(path);
      expect(data.lengthInBytes, greaterThan(0), reason: path);
    }
  });
}
