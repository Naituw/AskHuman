import Foundation
import XCTest

/// Where the real Mac binary and the local broker live; the tests skip themselves without them.
struct E2EEnvironment {
    var askHumanBinary: String
    var brokerDirectory: URL
    var node: String

    static func current() throws -> E2EEnvironment {
        let env = ProcessInfo.processInfo.environment
        guard let bin = env["AH_E2E_BIN"], !bin.isEmpty else {
            throw XCTSkip("AH_E2E_BIN is not set (path of the installed AskHuman binary)")
        }
        guard let dir = env["AH_E2E_BROKER_DIR"], !dir.isEmpty else {
            throw XCTSkip("AH_E2E_BROKER_DIR is not set (ios/Tools/local-broker, after npm install)")
        }
        return E2EEnvironment(
            askHumanBinary: bin,
            brokerDirectory: URL(fileURLWithPath: dir),
            node: env["AH_E2E_NODE"] ?? "/opt/homebrew/bin/node"
        )
    }
}

/// The local MQTT broker (Node, aedes) on a free loopback port.
final class LocalBroker: @unchecked Sendable {
    let username = "e2e"
    let password = "pw-" + UUID().uuidString.prefix(8).lowercased()
    private(set) var port = 0
    private let cmd: Cmd

    init(environment: E2EEnvironment) async throws {
        cmd = Cmd(
            environment.node, ["broker.js", "--port", "0", "--user", "e2e:\(password)"],
            env: ["PATH": ProcessInfo.processInfo.environment["PATH"] ?? ""], cwd: environment.brokerDirectory
        )
        try cmd.start()
        guard let line = await cmd.waitForLine(timeout: 15, { $0.hasPrefix("READY ") }),
              let data = line.dropFirst(6).data(using: .utf8),
              let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let port = json["port"] as? Int
        else {
            cmd.terminate()
            throw E2EError("the local broker did not start: \(cmd.stderr)")
        }
        self.port = port
    }

    func stop() { cmd.terminate() }
}

/// A throwaway Mac: its own config directory (so its own daemon, socket and config), no keychain,
/// popup off.
///
/// The instance MUST be isolated through `ASKHUMAN_HOME`, not through `HOME` alone. On macOS the
/// daemon is started through a launchd job whose label is global unless `ASKHUMAN_HOME` is set (a
/// Dev Instance gets its own label); a HOME-only sandbox therefore boots out the user's real daemon
/// and any in-flight request with it.
final class MacInstance: @unchecked Sendable {
    let home: URL
    let configDir: URL
    let environment: E2EEnvironment
    let env: [String: String]
    private var children: [Cmd] = []

    init(environment: E2EEnvironment, broker: LocalBroker, macName: String = "E2E Mac") async throws {
        self.environment = environment
        home = FileManager.default.temporaryDirectory.appendingPathComponent("ah-e2e-\(UUID().uuidString.prefix(8))")
        configDir = home.appendingPathComponent("home")
        try FileManager.default.createDirectory(at: configDir, withIntermediateDirectories: true)
        env = [
            "HOME": home.path,
            "ASKHUMAN_HOME": configDir.path,
            "ASKHUMAN_NO_KEYCHAIN": "1",
            "PATH": ProcessInfo.processInfo.environment["PATH"] ?? "/usr/bin:/bin",
            "LANG": "en_US.UTF-8",
        ]
        precondition(env["ASKHUMAN_HOME"]?.hasPrefix(FileManager.default.temporaryDirectory.path) == true)
        // The iOS channel is hidden behind the experimental switch; the popup stays closed so that
        // requests go to the phone only; plain TCP is allowed for the loopback broker.
        let seed: [String: Any] = [
            "experimental": ["enabled": true],
            "channels": [
                "popup": ["enabled": false],
                "ios": ["insecureLocal": true],
            ],
        ]
        try JSONSerialization.data(withJSONObject: seed).write(to: configDir.appendingPathComponent("config.json"))
        let set = try await run(
            environment.askHumanBinary,
            [
                "channel", "set", "ios", "--enable", "--host", "127.0.0.1", "--port", "\(broker.port)", "--username", broker.username,
                "--relay-url", "http://127.0.0.1:9", "--mac-name", macName, "--password-stdin",
            ],
            env: env, cwd: home, stdin: broker.password
        )
        guard set.status == 0 else { throw E2EError("channel set ios failed: \(set.stderr) \(set.stdout)") }
        try await waitForBroker()
    }

    @discardableResult
    func cli(_ args: [String], stdin: String? = nil, timeout: TimeInterval = 60) async throws -> CmdResult {
        try await run(environment.askHumanBinary, args, env: env, cwd: home, stdin: stdin, timeout: timeout)
    }

    func json(_ args: [String]) async throws -> [String: Any] {
        let result = try await cli(args)
        guard result.status == 0, let data = result.stdout.data(using: .utf8),
              let object = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { throw E2EError("bad JSON from \(args.joined(separator: " ")): \(result.stderr) \(result.stdout.prefix(200))") }
        return object
    }

    func jsonArray(_ args: [String]) async throws -> [[String: Any]] {
        let result = try await cli(args)
        guard result.status == 0, let data = result.stdout.data(using: .utf8),
              let array = try JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else { throw E2EError("bad JSON array from \(args.joined(separator: " ")): \(result.stderr) \(result.stdout.prefix(200))") }
        return array
    }

    func status() async throws -> [String: Any] { try await json(["channel", "ios", "status", "--json"]) }

    func waitForBroker() async throws {
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if let status = try? await status(), status["connected"] as? Bool == true { return }
            try await Task.sleep(nanoseconds: 300_000_000)
        }
        throw E2EError("the Mac never connected to the local broker")
    }

    /// Starts a pairing and returns the pairing code the phone takes in.
    func startPairing() async throws -> String {
        let cmd = Cmd(environment.askHumanBinary, ["channel", "ios", "pair"], env: env, cwd: home)
        try cmd.start()
        children.append(cmd)
        guard let line = await cmd.waitForLine(timeout: 30, { $0.hasPrefix("ahpair1.") }) else {
            throw E2EError("no pairing code: \(cmd.stderr.suffix(300))")
        }
        return line.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Starts `AskHuman` with the given arguments and returns the running process.
    func ask(_ args: [String], stdin: String? = nil) throws -> Cmd {
        let cmd = Cmd(environment.askHumanBinary, args, env: env, cwd: home, stdin: stdin)
        try cmd.start()
        children.append(cmd)
        return cmd
    }

    func stop() async {
        for child in children { child.terminate() }
        _ = try? await cli(["daemon", "stop"], timeout: 20)
        await releaseSystemResources()
        try? FileManager.default.removeItem(at: home)
    }

    /// `daemon stop` leaves things behind: the launchd job (loaded, not running) and the GUI host.
    private func releaseSystemResources() async {
        // Only ever an instance job: the production daemon's plist has no ".instance." in its label.
        let plist = configDir.appendingPathComponent("daemon-launchd.plist")
        if let text = try? String(contentsOf: plist, encoding: .utf8), text.contains(".daemon.instance.") {
            _ = try? await run("/bin/launchctl", ["bootout", "gui/\(getuid())", plist.path], env: [:], cwd: home, timeout: 20)
        }
        // Helper processes (the GUI host above all) can start late; sweep a few times. A process is
        // ours only if it carries this instance's throwaway ASKHUMAN_HOME.
        for round in 0..<4 {
            let pids = await processes(carrying: "ASKHUMAN_HOME=\(configDir.path)")
            if pids.isEmpty { return }
            for pid in pids { kill(pid, round < 2 ? SIGTERM : SIGKILL) }
            try? await Task.sleep(nanoseconds: 300_000_000)
        }
    }

    /// Processes of the AskHuman binary under test whose environment contains `marker`. (`ps e` only
    /// shows an environment when it is asked about one process at a time.)
    private func processes(carrying marker: String) async -> [Int32] {
        guard let list = try? await run("/bin/ps", ["-axo", "pid=,command="], env: [:], cwd: home, timeout: 20) else { return [] }
        var found: [Int32] = []
        for line in list.stdout.split(whereSeparator: \.isNewline) where line.contains(environment.askHumanBinary) {
            guard let pid = line.split(separator: " ").first.flatMap({ Int32($0) }) else { continue }
            if let detail = try? await run("/bin/ps", ["eww", "-p", "\(pid)"], env: [:], cwd: home, timeout: 20),
               detail.stdout.contains(marker) {
                found.append(pid)
            }
        }
        return found
    }
}
