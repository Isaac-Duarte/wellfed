import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/home/start_failed.dart';
import 'package:wellfed/state/fleet/app_state.dart';

/// The secure storage plugin's channel, answered here as a Keystore that
/// fails every read would be.
const _storage = MethodChannel('plugins.it_nomads.com/flutter_secure_storage');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  testWidgets("a client key the Keystore can't read stops the start and is never replaced", (tester) async {
    final calls = <MethodCall>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(_storage, (call) async {
      calls.add(call);
      if (call.method == 'read') throw PlatformException(code: 'Exception encountered', message: 'keystore');
      return null;
    });
    addTearDown(() => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(_storage, null));

    final AppStart start = await tester.runAsync(AppState.start) ?? fail('the start never ended');

    expect(start, isA<AppFailed>());
    expect(calls.map((call) => call.method), ['read'], reason: 'nothing written over the key, nothing deleted');
    final options = (calls.single.arguments as Map<Object?, Object?>)['options'] as Map<Object?, Object?>?;
    expect(options?['resetOnError'], 'false', reason: 'the plugin would otherwise delete the key itself');

    await tester.pumpWidget(StartFailedApp(message: (start as AppFailed).message));
    expect(find.textContaining('Nothing was deleted'), findsOneWidget);
  });
}
