import Cocoa
import CoreLocation
import CoreWLAN
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  /// Answers the install's question of which Wi-Fi an interface is on.
  private var wifi: WifiNames?
  /// Brings this window forward when an alert's notification is clicked
  /// (`lib/platform/app_window.dart`). macOS activates the app on a click
  /// by itself; this also restores a minimized window.
  private var windowChannel: FlutterMethodChannel?

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    // The page color of the appearance macOS is in, so a resize shows the
    // page rather than black: tokens.json's light and dark `background`. An
    // appearance chosen in the app's settings can still differ until
    // Flutter paints.
    flutterViewController.backgroundColor = NSColor(name: nil) { appearance in
      appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        ? NSColor(srgbRed: 0x12 / 255, green: 0x14 / 255, blue: 0x17 / 255, alpha: 1)
        : NSColor(srgbRed: 0xF4 / 255, green: 0xF6 / 255, blue: 0xFA / 255, alpha: 1)
    }
    self.contentViewController = flutterViewController

    // Room for the sidebar beside a page; below this the page's two columns
    // already fold into one.
    self.setContentSize(NSSize(width: 1200, height: 800))
    self.contentMinSize = NSSize(width: 760, height: 560)
    self.center()

    RegisterGeneratedPlugins(registry: flutterViewController)
    wifi = WifiNames(messenger: flutterViewController.engine.binaryMessenger)
    windowChannel = FlutterMethodChannel(
      name: "io.github.isaacduarte.wellfed/window",
      binaryMessenger: flutterViewController.engine.binaryMessenger)
    windowChannel?.setMethodCallHandler { [weak self] call, result in
      guard call.method == "front", let self else {
        result(FlutterMethodNotImplemented)
        return
      }
      NSApp.activate(ignoringOtherApps: true)
      if self.isMiniaturized { self.deminiaturize(nil) }
      self.makeKeyAndOrderFront(nil)
      result(nil)
    }

    super.awakeFromNib()
  }

  /// The close button and ⌘W quit the app, as closing its one window does
  /// anyway. Quitting goes through Flutter's exit request, so a run going in
  /// Dart can ask first and keep the window open.
  override func performClose(_ sender: Any?) {
    NSApp.terminate(sender)
  }
}

/// Which Wi-Fi a network interface is on, for the install's join of the
/// feeder's setup network (`lib/platform/wifi_name.dart`). macOS names the
/// Wi-Fi only to an app allowed the computer's location, so the first question
/// asks for that. The answer is `{"ssid": name}`, with no name when the
/// interface is on no Wi-Fi or location was refused: the join then refuses
/// to send anything.
final class WifiNames: NSObject, CLLocationManagerDelegate {
  private let location = CLLocationManager()
  private var waiting: [(Bool) -> Void] = []
  private let channel: FlutterMethodChannel

  init(messenger: FlutterBinaryMessenger) {
    channel = FlutterMethodChannel(
      name: "io.github.isaacduarte.wellfed/desktop-wifi", binaryMessenger: messenger)
    super.init()
    location.delegate = self
    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "ssid", let interface = call.arguments as? String, let self else {
        result(FlutterMethodNotImplemented)
        return
      }
      self.whenAllowed { allowed in
        let ssid = allowed ? CWWiFiClient.shared().interface(withName: interface)?.ssid() : nil
        result(["ssid": ssid as Any])
      }
    }
  }

  /// `then` once macOS says whether this app may know the location, asking
  /// the owner the first time.
  private func whenAllowed(_ then: @escaping (Bool) -> Void) {
    switch location.authorizationStatus {
    case .notDetermined:
      waiting.append(then)
      location.requestWhenInUseAuthorization()
    case let status:
      then(WifiNames.allows(status))
    }
  }

  /// Always, or when in use (raw value 4, which macOS's SDK does not name).
  private static func allows(_ status: CLAuthorizationStatus) -> Bool {
    status == .authorizedAlways || status.rawValue == 4
  }

  func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
    let status = manager.authorizationStatus
    guard status != .notDetermined else { return }
    let answered = waiting
    waiting = []
    for then in answered { then(WifiNames.allows(status)) }
  }
}
