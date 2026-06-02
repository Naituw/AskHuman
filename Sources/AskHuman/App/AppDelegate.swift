import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let onLaunch: () -> Void

    init(onLaunch: @escaping () -> Void) {
        self.onLaunch = onLaunch
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.activate(ignoringOtherApps: true)
        onLaunch()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        // 终止时机由具体流程控制（提问/设置）
        false
    }
}
