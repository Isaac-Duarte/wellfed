import 'package:flutter/services.dart';

/// The app's window on a computer, through each runner's own half
/// (`MainFlutterWindow.swift`, `my_application.cc`, `flutter_window.cpp`).
class AppWindow {
  static const _channel = MethodChannel('io.github.isaacduarte.wellfed/window');

  /// Bring the window forward, restored if it was minimized: a tap on an
  /// alert's notification opens its feeder's alerts there. A system may only
  /// flag the window rather than raise it, as a Wayland compositor does for
  /// an app that wasn't handed the focus.
  static Future<void> front() async {
    try {
      await _channel.invokeMethod<void>('front');
    } on MissingPluginException {
      // A runner with no window half: the page still opens behind.
    }
  }
}
