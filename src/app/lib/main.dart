import 'package:flutter/material.dart';

import 'package:wellfed/app.dart';
import 'package:wellfed/src/rust/frb_generated.dart';
import 'package:wellfed/state/fleet/app_state.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final AppState app = await AppState.load();
  runApp(WellfedApp(app: app));
}
