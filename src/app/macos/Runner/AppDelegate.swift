import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return true
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }

  /// Settings… in the app menu. The app owns its routes, so the pick goes to
  /// Dart (`lib/platform/menu.dart`), which opens its settings page.
  @IBAction func showSettings(_ sender: Any?) {
    guard let window = mainFlutterWindow,
      let controller = window.contentViewController as? FlutterViewController
    else { return }
    window.makeKeyAndOrderFront(nil)
    FlutterMethodChannel(name: "wellfed/menu", binaryMessenger: controller.engine.binaryMessenger)
      .invokeMethod("settings", arguments: nil)
  }
}
