import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
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

    super.awakeFromNib()
  }
}
