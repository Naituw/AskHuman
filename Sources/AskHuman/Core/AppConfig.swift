import Foundation

enum ThemeMode: String, Codable, CaseIterable {
    case system
    case light
    case dark
}

struct GeneralConfig: Codable {
    var theme: ThemeMode
    var alwaysOnTop: Bool

    init(theme: ThemeMode = .system, alwaysOnTop: Bool = false) {
        self.theme = theme
        self.alwaysOnTop = alwaysOnTop
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        theme = (try? c.decodeIfPresent(ThemeMode.self, forKey: .theme)) ?? .system
        alwaysOnTop = (try? c.decodeIfPresent(Bool.self, forKey: .alwaysOnTop)) ?? false
    }
}

struct PopupChannelConfig: Codable {
    var enabled: Bool
    var width: Double
    var height: Double

    init(enabled: Bool = true, width: Double = 560, height: Double = 620) {
        self.enabled = enabled
        self.width = width
        self.height = height
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let def = PopupChannelConfig()
        enabled = (try? c.decodeIfPresent(Bool.self, forKey: .enabled)) ?? def.enabled
        width = (try? c.decodeIfPresent(Double.self, forKey: .width)) ?? def.width
        height = (try? c.decodeIfPresent(Double.self, forKey: .height)) ?? def.height
    }
}

struct TelegramChannelConfig: Codable {
    var enabled: Bool
    var botToken: String
    var chatId: String
    var apiBaseUrl: String

    init(
        enabled: Bool = false,
        botToken: String = "",
        chatId: String = "",
        apiBaseUrl: String = "https://api.telegram.org"
    ) {
        self.enabled = enabled
        self.botToken = botToken
        self.chatId = chatId
        self.apiBaseUrl = apiBaseUrl
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let def = TelegramChannelConfig()
        enabled = (try? c.decodeIfPresent(Bool.self, forKey: .enabled)) ?? def.enabled
        botToken = (try? c.decodeIfPresent(String.self, forKey: .botToken)) ?? def.botToken
        chatId = (try? c.decodeIfPresent(String.self, forKey: .chatId)) ?? def.chatId
        apiBaseUrl = (try? c.decodeIfPresent(String.self, forKey: .apiBaseUrl)) ?? def.apiBaseUrl
    }
}

struct ChannelsConfig: Codable {
    var popup: PopupChannelConfig
    var telegram: TelegramChannelConfig

    init(popup: PopupChannelConfig = PopupChannelConfig(), telegram: TelegramChannelConfig = TelegramChannelConfig()) {
        self.popup = popup
        self.telegram = telegram
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        popup = (try? c.decodeIfPresent(PopupChannelConfig.self, forKey: .popup)) ?? PopupChannelConfig()
        telegram = (try? c.decodeIfPresent(TelegramChannelConfig.self, forKey: .telegram)) ?? TelegramChannelConfig()
    }
}

struct AppConfig: Codable {
    var general: GeneralConfig
    var channels: ChannelsConfig

    init(general: GeneralConfig = GeneralConfig(), channels: ChannelsConfig = ChannelsConfig()) {
        self.general = general
        self.channels = channels
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        general = (try? c.decodeIfPresent(GeneralConfig.self, forKey: .general)) ?? GeneralConfig()
        channels = (try? c.decodeIfPresent(ChannelsConfig.self, forKey: .channels)) ?? ChannelsConfig()
    }
}

enum ConfigStore {
    /// 读取配置；文件不存在或损坏时返回默认配置
    static func load() -> AppConfig {
        let url = Paths.configFile
        guard let data = try? Data(contentsOf: url) else {
            return AppConfig()
        }
        let decoder = JSONDecoder()
        if let config = try? decoder.decode(AppConfig.self, from: data) {
            return config
        }
        return AppConfig()
    }

    /// 原子写入配置
    @discardableResult
    static func save(_ config: AppConfig) -> Bool {
        let dir = Paths.configDir
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            let data = try encoder.encode(config)
            let tmp = dir.appendingPathComponent("config.json.tmp-\(UUID().uuidString)")
            try data.write(to: tmp, options: .atomic)
            let dst = Paths.configFile
            if FileManager.default.fileExists(atPath: dst.path) {
                _ = try? FileManager.default.replaceItemAt(dst, withItemAt: tmp)
                if FileManager.default.fileExists(atPath: tmp.path) {
                    try? FileManager.default.removeItem(at: tmp)
                }
            } else {
                try FileManager.default.moveItem(at: tmp, to: dst)
            }
            return true
        } catch {
            return false
        }
    }
}
