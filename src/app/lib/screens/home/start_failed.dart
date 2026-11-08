import 'package:flutter/material.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/layout.dart';

/// The app in place of Wellfed when its start failed: the phone's storage,
/// not the contents of a file, which are set aside instead. Says so rather
/// than leave a blank screen.
class StartFailedApp extends StatelessWidget {
  const StartFailedApp({super.key, required this.message});

  /// What failed, for a support request.
  final String message;

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'Wellfed',
    debugShowCheckedModeBanner: false,
    theme: wellfedTheme(Brightness.light),
    darkTheme: wellfedTheme(Brightness.dark),
    home: Builder(
      builder: (context) => StepScaffold(
        leading: Leading.none,
        title: "Wellfed couldn't start",
        body:
            "This $deviceName's storage couldn't be read. Nothing was changed or deleted. "
            'Close the app and open it again; if this stays, restart the $deviceName.',
        content: [
          SelectableText(
            message,
            style: TextStyle(fontSize: Sizes.sm, color: context.colors.textMuted),
          ),
        ],
      ),
    ),
  );
}
