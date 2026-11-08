import 'package:flutter/services.dart';

/// The menu bar's items that act inside the app. Only the Mac has one: its
/// runner sends each pick over a method channel.
abstract interface class MenuPorts {
  /// Call `settings` when the owner picks Settings… (⌘,), until the returned
  /// function stops it. A later [listen] takes over, and stopping an earlier
  /// one then leaves the later one listening.
  VoidCallback listen({required VoidCallback settings});
}

/// [MenuPorts] on a Mac, over the channel `macos/Runner/AppDelegate.swift`
/// sends the menu's picks on.
class AppMenuPorts implements MenuPorts {
  /// The Mac's menu bar.
  const AppMenuPorts();

  static const _channel = MethodChannel('wellfed/menu');

  /// The listener the channel calls now. The channel has one handler, so
  /// a window built again (a hot restart, a test) replaces the last one's.
  static Object? _current;

  @override
  VoidCallback listen({required VoidCallback settings}) {
    final token = Object();
    _current = token;
    _channel.setMethodCallHandler((call) async {
      if (call.method == 'settings') settings();
    });

    return () {
      if (!identical(_current, token)) return;
      _current = null;
      _channel.setMethodCallHandler(null);
    };
  }
}
