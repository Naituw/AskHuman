import Foundation

enum Paths {
    /// 用户主目录
    static var home: URL {
        FileManager.default.homeDirectoryForCurrentUser
    }

    /// 配置目录 ~/.humaninloop
    static var configDir: URL {
        home.appendingPathComponent(".humaninloop", isDirectory: true)
    }

    /// 配置文件 ~/.humaninloop/config.json
    static var configFile: URL {
        configDir.appendingPathComponent("config.json", isDirectory: false)
    }

    /// 本次请求的图片落盘目录 temp/humaninloop/<requestId>/
    static func requestTempDir(requestId: String) -> URL {
        let base = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
            .appendingPathComponent("humaninloop", isDirectory: true)
            .appendingPathComponent(requestId, isDirectory: true)
        return base
    }

    /// Cursor 相关路径
    static var cursorDir: URL {
        home.appendingPathComponent(".cursor", isDirectory: true)
    }

    static var cursorHooksJSON: URL {
        cursorDir.appendingPathComponent("hooks.json", isDirectory: false)
    }

    static var cursorHookScript: URL {
        cursorDir
            .appendingPathComponent("hooks", isDirectory: true)
            .appendingPathComponent("humaninloop-timeout.sh", isDirectory: false)
    }
}
