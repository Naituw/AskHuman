import AppKit
import SwiftUI

@MainActor
final class SettingsViewModel: ObservableObject {
    @Published var config: AppConfig {
        didSet {
            ConfigStore.save(config)
            AppBootstrap.applyTheme(config.general.theme)
        }
    }

    @Published var hookInstalled: Bool
    @Published var hooksJSONExists: Bool
    @Published var hookMessage: String?
    @Published var hookError: Bool = false

    @Published var telegramTesting: Bool = false
    @Published var telegramMessage: String?
    @Published var telegramError: Bool = false

    @Published var promptCopied: Bool = false

    init() {
        config = ConfigStore.load()
        hookInstalled = CursorHook.isInstalled()
        hooksJSONExists = CursorHook.hooksJSONExists
    }

    // MARK: - Cursor Hook

    func installHook() {
        do {
            hookMessage = try CursorHook.install()
            hookError = false
        } catch {
            hookMessage = "操作失败：\(error)"
            hookError = true
        }
        refreshHookStatus()
    }

    func uninstallHook() {
        do {
            hookMessage = try CursorHook.uninstall()
            hookError = false
        } catch {
            hookMessage = "操作失败：\(error)"
            hookError = true
        }
        refreshHookStatus()
    }

    func revealHooks() {
        CursorHook.revealHooksJSON()
    }

    func refreshHookStatus() {
        hookInstalled = CursorHook.isInstalled()
        hooksJSONExists = CursorHook.hooksJSONExists
    }

    // MARK: - 提示词

    func copyPrompt() {
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(Prompts.cliReference, forType: .string)
        promptCopied = true
        Task {
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            promptCopied = false
        }
    }

    // MARK: - Telegram 测试

    func testTelegram() {
        telegramTesting = true
        telegramMessage = nil
        let cfg = config.channels.telegram
        Task {
            do {
                let client = try TelegramClient(
                    token: cfg.botToken,
                    chatIdString: cfg.chatId,
                    apiBaseUrl: cfg.apiBaseUrl
                )
                let msg = try await client.testConnection()
                telegramMessage = msg
                telegramError = false
            } catch {
                telegramMessage = "\(error)"
                telegramError = true
            }
            telegramTesting = false
        }
    }
}
