import Foundation

/// `state/todo-sync/helper.json`: the hand-over from the core to the helper (plan §2). The core owns it;
/// the helper only reads it, at launch and whenever the file changes.
struct HelperConfig: Equatable {
    /// RPC protocol the core speaks; informational (the real handshake is `hello`).
    var protocolVersion: Int = Wire.protocolVersion
    /// Absolute path of the `AskHuman` executable the helper starts for every tick.
    var askhuman: String
    /// `~/.askhuman/state/todo-sync`: holds `helper.sock` and `state.json`.
    var stateDir: String
    /// The todo file the core keeps; the helper watches it so a local edit reaches the cloud within seconds.
    var todosPath: String
    var pollMinutes: Int = HelperConfig.defaultPollMinutes
    /// False while sync is turned off: the helper stays up (so the core can still say hello) but idles.
    var enabled: Bool = true

    static let defaultPollMinutes = 10
    static let minPollMinutes = 1
    static let maxPollMinutes = 24 * 60

    static func parse(_ data: Data) -> HelperConfig? {
        guard let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
            let askhuman = object["askhuman"] as? String, !askhuman.isEmpty,
            let stateDir = object["stateDir"] as? String, !stateDir.isEmpty,
            let todosPath = object["todosPath"] as? String, !todosPath.isEmpty
        else { return nil }
        var config = HelperConfig(askhuman: askhuman, stateDir: stateDir, todosPath: todosPath)
        if let p = (object["protocol"] as? NSNumber)?.intValue { config.protocolVersion = p }
        if let m = (object["pollMinutes"] as? NSNumber)?.intValue {
            config.pollMinutes = min(max(m, minPollMinutes), maxPollMinutes)
        }
        if let e = object["enabled"] as? Bool { config.enabled = e }
        return config
    }

    static func load(from url: URL) -> HelperConfig? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return parse(data)
    }
}

/// Where the helper keeps its files. Everything is derived from one directory so tests can redirect it.
struct HelperPaths: Equatable {
    var stateDir: URL

    var configURL: URL { stateDir.appendingPathComponent("helper.json") }
    var socketURL: URL { stateDir.appendingPathComponent("helper.sock") }
    var lockURL: URL { stateDir.appendingPathComponent("helper.lock") }

    /// `$ASKHUMAN_TODO_SYNC_DIR`, else `~/.askhuman/state/todo-sync`.
    static func resolve(environment: [String: String] = ProcessInfo.processInfo.environment, home: URL = FileManager.default.homeDirectoryForCurrentUser) -> HelperPaths {
        if let override = environment["ASKHUMAN_TODO_SYNC_DIR"], !override.isEmpty {
            return HelperPaths(stateDir: URL(fileURLWithPath: override, isDirectory: true))
        }
        return HelperPaths(
            stateDir: home.appendingPathComponent(".askhuman/state/todo-sync", isDirectory: true))
    }
}
