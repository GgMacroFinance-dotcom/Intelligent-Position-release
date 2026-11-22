import Cocoa
import XCTest
@testable import intelligent_position

class RunnerTests: XCTestCase {

  @MainActor
  func testFirstLaunchFitsVisibleScreen() throws {
    let window = makeWindow()
    let visible = try XCTUnwrap(window.screen ?? NSScreen.main).visibleFrame
    let name = UUID().uuidString
    defer { NSWindow.removeFrame(usingName: name) }

    window.restoreMainFrame(named: name)
    let preferred = window.frameRect(forContentRect: NSRect(x: 0, y: 0, width: 1120, height: 760))
    XCTAssertEqual(window.frame.width, min(preferred.width, visible.width), accuracy: 1)
    XCTAssertEqual(window.frame.height, min(preferred.height, visible.height), accuracy: 1)
    XCTAssertEqual(window.frame.midX, visible.midX, accuracy: 1)
    XCTAssertEqual(window.frame.midY, visible.midY, accuracy: 1)
    XCTAssertEqual(window.frameAutosaveName, name)
  }

  @MainActor
  func testRestoresSavedSizeAndPosition() throws {
    let first = makeWindow()
    let visible = try XCTUnwrap(first.screen ?? NSScreen.main).visibleFrame
    let saved = visible.insetBy(dx: 64, dy: 80).integral
    let name = UUID().uuidString
    defer { NSWindow.removeFrame(usingName: name) }
    first.setFrame(saved, display: false)
    first.saveFrame(usingName: name)

    let next = makeWindow()
    next.restoreMainFrame(named: name)
    XCTAssertEqual(next.frame, first.frame)
  }

  @MainActor
  private func makeWindow() -> MainFlutterWindow {
    MainFlutterWindow(
      contentRect: NSRect(x: 100, y: 100, width: 800, height: 600),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered, defer: false)
  }

}
