import 'dart:io';

import 'package:flutter/material.dart';

import 'package:wellfed/app.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/platform/host.dart';
import 'package:wellfed/screens/home/start_failed.dart';
import 'package:wellfed/src/rust/frb_generated.dart';
import 'package:wellfed/state/fleet/app_state.dart';

// The background engine's entry point (BackgroundEngine.kt) is looked up by
// its library, and a library nothing reaches from here isn't compiled in: the
// engine then can't start, and no catch-up or instant alert runs while the
// app is closed. test/platform/background_entry_test.dart checks this holds.
export 'package:wellfed/platform/background.dart' show background;

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  await nameDataDirectory();

  // The app's key first, then alerts. Asking for the catch-up can start it at
  // once in a background engine with its own secure storage, and two engines
  // opening Android's secure storage before it holds a key each make one:
  // the second overwrites the first, and the next start can't read the
  // client key. So no background engine starts until this one has read or
  // made the key, and a start that failed starts none.
  final AppStart start = await AppState.start();
  if (start is AppStarted) {
    if (Platform.isAndroid) await startAlerts();
    if (onDesktopHost) await startComputerAlerts();
  }

  runApp(switch (start) {
    AppStarted(:final AppState app) => WellfedApp(app: app, alerts: Platform.isAndroid || onDesktopHost),
    AppFailed(:final String message) => StartFailedApp(message: message),
  });
}
