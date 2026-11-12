import 'dart:ui';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/platform/notices.dart';
import 'package:wellfed/platform/secure_storage.dart';
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/src/rust/frb_generated.dart';

/// The channel to the Kotlin engine this runs in (`BackgroundEngine.kt`).
const _channel = MethodChannel('io.github.isaacduarte.wellfed/background');

/// Entry point of a background engine: the periodic catch-up's or the instant
/// service's. That engine has only the notification, secure storage and NSD
/// plugins, and never the Bluetooth one, so nothing here may reach any other.
/// Kotlin asks for a round at a time and hears when it is done.
@pragma('vm:entry-point')
Future<void> background() async {
  WidgetsFlutterBinding.ensureInitialized();
  DartPluginRegistrant.ensureInitialized();
  await RustLib.init();
  await Notices.init();

  late final String dir;
  _channel.setMethodCallHandler((call) async {
    if (call.method != 'round') throw MissingPluginException(call.method);
    await _round(dir);
    return null;
  });
  // Kotlin answers with the app's directory, as path_provider would, whose
  // Android half this engine leaves out.
  dir = await _channel.invokeMethod<String>('ready') ?? '';
}

/// One round with every feeder this phone knows, through the owner's server
/// for those away from home. With no client key or no feeders there is
/// nobody to catch up with.
Future<void> _round(String dir) async {
  final String? secret = await secureStorage.read(key: clientSecretName);
  if (secret == null || dir.isEmpty) return;

  final AppStore store = await loadStore(path: '$dir/wellfed.json');
  await catchUpRound(
    key: ClientKey.restore(secret: identityBytes(hex: secret)),
    dir: dir,
    feeders: store.feeders,
    network: () async => phoneNetwork(await _channel.invokeMapMethod<Object?, Object?>('network')),
    failures: RoundFailures.logged,
    server: await alertServer(dir),
  );
}
