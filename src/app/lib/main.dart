import 'package:flutter/material.dart';

import 'package:wellfed/app.dart';
import 'package:wellfed/screens/home/start_failed.dart';
import 'package:wellfed/src/rust/frb_generated.dart';
import 'package:wellfed/state/fleet/app_state.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  runApp(switch (await AppState.start()) {
    AppStarted(:final AppState app) => WellfedApp(app: app),
    AppFailed(:final String message) => StartFailedApp(message: message),
  });
}
