import 'dart:io';

import 'package:flutter/material.dart';

import 'package:wellfed/app.dart';
import 'package:wellfed/platform/alert_ports.dart';
import 'package:wellfed/platform/host.dart';
import 'package:wellfed/screens/home/start_failed.dart';
import 'package:wellfed/src/rust/frb_generated.dart';
import 'package:wellfed/state/fleet/app_state.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  if (Platform.isAndroid) await startAlerts();
  if (onDesktopHost) await startComputerAlerts();
  runApp(switch (await AppState.start()) {
    AppStarted(:final AppState app) => WellfedApp(app: app, alerts: Platform.isAndroid || onDesktopHost),
    AppFailed(:final String message) => StartFailedApp(message: message),
  });
}
