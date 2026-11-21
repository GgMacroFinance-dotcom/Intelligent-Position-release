import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    self.contentViewController = flutterViewController
    restoreMainFrame(named: "IPMainWindow")
    RegisterGeneratedPlugins(registry: flutterViewController)
    super.awakeFromNib()
  }

  func restoreMainFrame(named name: NSWindow.FrameAutosaveName) {
    let restored = setFrameUsingName(name)
    setFrameAutosaveName(name)
    let titleBar = NSRect(x: frame.minX, y: frame.maxY - 22, width: frame.width, height: 22)
    let reachable = NSScreen.screens.contains {
      let overlap = $0.visibleFrame.intersection(titleBar)
      return overlap.width >= 80 && overlap.height >= 12
    }
    if (!restored || !reachable), let visible = (screen ?? NSScreen.main)?.visibleFrame {
      let preferred = frameRect(forContentRect: NSRect(x: 0, y: 0, width: 1120, height: 760))
      let size = restored ? frame.size : preferred.size
      let width = min(size.width, visible.width)
      let height = min(size.height, visible.height)
      // 拔掉显示器后找回窗口；仍能拖动的跨屏窗口保留用户的位置。
      let fitted = NSRect(
        x: visible.midX - width / 2, y: visible.midY - height / 2,
        width: width, height: height
      )
      setFrame(fitted, display: false)
    }
  }
}
