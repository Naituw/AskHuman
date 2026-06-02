import AppKit
import SwiftUI

@MainActor
final class PopupWindowController: NSObject, NSWindowDelegate {
    private var window: NSWindow?
    private let viewModel: PopupViewModel
    private let config: AppConfig
    private var closingProgrammatically = false

    init(viewModel: PopupViewModel, config: AppConfig) {
        self.viewModel = viewModel
        self.config = config
    }

    func show() {
        let hosting = NSHostingController(rootView: PopupView(viewModel: viewModel))
        let window = NSWindow(contentViewController: hosting)
        window.title = "HumanInLoop"
        window.styleMask = [.titled, .closable, .resizable, .miniaturizable]
        window.setContentSize(NSSize(
            width: config.channels.popup.width,
            height: config.channels.popup.height
        ))
        window.delegate = self
        if config.general.alwaysOnTop {
            window.level = .floating
        }
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        self.window = window
    }

    /// 被其他 Channel 抢答时静默关闭
    func closeSilently() {
        viewModel.markResolvedSilently()
        closingProgrammatically = true
        window?.close()
        window = nil
    }

    func windowWillClose(_ notification: Notification) {
        guard !closingProgrammatically else { return }
        // 用户点了红色关闭按钮，视为取消
        viewModel.cancel()
    }
}
