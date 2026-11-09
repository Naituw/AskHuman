import Darwin
import Foundation

/// Starts `AskHuman todo-sync tick --reason <reason>` and reports its exit status. The helper never
/// merges or writes todos itself: all sync logic lives in the core, the helper only wakes it up.
final class TickProcessRunner {
    /// A tick that has not finished after this long is killed; the scheduler then moves on.
    static let timeoutSeconds: TimeInterval = 120

    private let fixture: SystemFixture?
    private let environment: [String: String]
    private let probeTimeout: TimeInterval
    private let log: (String, [String: Any]) -> Void

    init(
        fixture: SystemFixture? = nil,
        environment: [String: String] = ProcessInfo.processInfo.environment,
        probeTimeout: TimeInterval = 3,
        log: @escaping (String, [String: Any]) -> Void
    ) {
        self.fixture = fixture
        self.environment = environment
        self.probeTimeout = probeTimeout
        self.log = log
    }

    func run(config: HelperConfig?, reason: TickScheduler.Reason, done: @escaping (Int32?) -> Void) {
        guard let config, config.enabled else {
            done(nil)
            return
        }
        guard FileManager.default.isExecutableFile(atPath: config.askhuman) else {
            log("tick_not_started", ["why": "askhuman missing"])
            done(nil)
            return
        }
        if let fixture {
            // Probe every tick, including after an in-place replacement of the executable.
            DispatchQueue.global(qos: .utility).async {
                do {
                    try fixture.validateConfiguration(config)
                    try self.probe(config: config, fixture: fixture)
                    let environment = try fixture.tickEnvironment(self.environment, config: config)
                    self.spawn(config: config, reason: reason, environment: environment,
                               directory: fixture.accountHome, done: done)
                } catch {
                    self.log("tick_not_started", ["why": "\(error)"])
                    done(nil)
                }
            }
        } else {
            var environment = self.environment
            environment["ASKHUMAN_TODO_SYNC_CALLER"] = "helper"
            spawn(config: config, reason: reason, environment: environment,
                  directory: FileManager.default.homeDirectoryForCurrentUser, done: done)
        }
    }

    private func probe(config: HelperConfig, fixture: SystemFixture) throws {
        let outputURL = fixture.paths.stateDir.appendingPathComponent("fixture-probe-\(UUID().uuidString).json")
        let fd = Darwin.open(outputURL.path, O_CREAT | O_EXCL | O_RDWR | O_CLOEXEC | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw FixtureError("cannot create private capability probe output") }
        let output = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer {
            try? output.close()
            try? FileManager.default.removeItem(at: outputURL)
        }
        let probe = Process()
        probe.executableURL = URL(fileURLWithPath: config.askhuman)
        probe.arguments = ["--development-capabilities"]
        probe.environment = fixture.probeEnvironment(environment)
        probe.currentDirectoryURL = fixture.accountHome
        probe.standardInput = FileHandle.nullDevice
        probe.standardOutput = output
        probe.standardError = FileHandle.nullDevice
        try probe.run()
        let started = ProcessInfo.processInfo.systemUptime
        while probe.isRunning {
            var info = stat()
            let oversized = fstat(fd, &info) == 0 && info.st_size > 16 * 1024
            if oversized || ProcessInfo.processInfo.systemUptime - started >= probeTimeout {
                kill(probe.processIdentifier, SIGKILL)
                probe.waitUntilExit()
                throw FixtureError(oversized ? "fixture capability output is too large" : "fixture capability probe timed out")
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        try output.seek(toOffset: 0)
        let bytes = try output.read(upToCount: 16 * 1024 + 1) ?? Data()
        guard probe.terminationStatus == 0, SystemFixture.supportsCapabilities(bytes) else {
            throw FixtureError("AskHuman does not support system fixture v2")
        }
    }

    private func spawn(
        config: HelperConfig, reason: TickScheduler.Reason, environment: [String: String],
        directory: URL, done: @escaping (Int32?) -> Void
    ) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: config.askhuman)
        process.arguments = ["todo-sync", "tick", "--reason", reason.rawValue]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        // launchd starts agents in `/`; a tick should not resolve anything relative to that.
        process.currentDirectoryURL = directory
        process.environment = environment

        let timeout = DispatchWorkItem { [weak process] in
            guard let process, process.isRunning else { return }
            self.log("tick_timeout", ["reason": reason.rawValue])
            process.terminate()
        }
        process.terminationHandler = { finishedProcess in
            timeout.cancel()
            done(finishedProcess.terminationStatus)
        }
        do {
            try process.run()
            DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + Self.timeoutSeconds, execute: timeout)
        } catch {
            log("tick_not_started", ["why": "spawn failed"])
            done(nil)
        }
    }
}
