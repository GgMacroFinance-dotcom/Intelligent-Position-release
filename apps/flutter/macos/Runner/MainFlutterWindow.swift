import Cocoa
import FlutterMacOS
import ServiceManagement

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    self.contentViewController = flutterViewController
    restoreMainFrame(named: "IPMainWindow")
    RegisterGeneratedPlugins(registry: flutterViewController)
    registerAutostart(messenger: flutterViewController.engine.binaryMessenger)
    super.awakeFromNib()
  }

  /// 开机启动（F-17）：登录时打开这个 App，用 SMAppService，要 macOS 13 以上。
  /// 在“系统设置 > 通用 > 登录项”里能看到、能关。Dart 那边是 lib/src/desktop/autostart.dart。
  func registerAutostart(messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "ip_app/autostart", binaryMessenger: messenger)
    channel.setMethodCallHandler { call, result in
      guard #available(macOS 13.0, *) else {
        result(call.method == "available" ? false : FlutterError(code: "unavailable", message: "要 macOS 13 以上", details: nil))
        return
      }
      let service = SMAppService.mainApp
      switch call.method {
      case "available":
        result(true)
      case "isEnabled":
        result(service.status == .enabled)
      case "setEnabled":
        let on = (call.arguments as? [String: Any])?["on"] as? Bool ?? false
        do {
          if on { try service.register() } else { try service.unregister() }
          result(nil)
        } catch {
          result(FlutterError(code: "failed", message: error.localizedDescription, details: nil))
        }
      default:
        result(FlutterMethodNotImplemented)
      }
    }
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
