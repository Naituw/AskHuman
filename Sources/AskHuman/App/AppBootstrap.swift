import AppKit
import Darwin

/// 启动并管理 NSApplication 生命周期
@MainActor
enum AppBootstrap {
    private static var delegate: AppDelegate?

    /// 启动 NSApplication，`onLaunch` 在启动完成后于主线程回调一次。
    /// 该方法阻塞直到 `stop()` 被调用。
    static func run(theme: ThemeMode, onLaunch: @escaping () -> Void) {
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)
        applyTheme(theme)
        let delegate = AppDelegate(onLaunch: onLaunch)
        self.delegate = delegate
        app.delegate = delegate

        // 屏蔽 AppKit/TSM 在终端运行时输出的系统噪音日志（如 TSM AdjustCapsLockLED）。
        // 仅在 run loop 期间重定向 stderr，结束后恢复，不影响我们自己的报错输出。
        let savedStderr = dup(STDERR_FILENO)
        let devnull = open("/dev/null", O_WRONLY)
        if devnull != -1 {
            dup2(devnull, STDERR_FILENO)
            close(devnull)
        }

        app.run()

        fflush(stderr)
        if savedStderr != -1 {
            dup2(savedStderr, STDERR_FILENO)
            close(savedStderr)
        }
    }

    /// 退出 run loop
    static func stop() {
        NSApp.stop(nil)
        // 投递一个空事件唤醒 run loop，确保 stop 立即生效
        let event = NSEvent.otherEvent(
            with: .applicationDefined,
            location: .zero,
            modifierFlags: [],
            timestamp: 0,
            windowNumber: 0,
            context: nil,
            subtype: 0,
            data1: 0,
            data2: 0
        )
        if let event {
            NSApp.postEvent(event, atStart: true)
        }
    }

    /// 应用主题到 NSApp.appearance
    static func applyTheme(_ theme: ThemeMode) {
        switch theme {
        case .system:
            NSApp.appearance = nil
        case .light:
            NSApp.appearance = NSAppearance(named: .aqua)
        case .dark:
            NSApp.appearance = NSAppearance(named: .darkAqua)
        }
    }
}
