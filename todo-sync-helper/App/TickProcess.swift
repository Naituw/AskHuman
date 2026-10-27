import Foundation

/// Starts `AskHuman todo-sync tick --reason <reason>` and reports its exit status. The helper never
/// merges or writes todos itself: all sync logic lives in the core, the helper only wakes it up.
final class TickProcessRunner {
    /// A tick that has not finished after this long is killed; the scheduler then moves on.
    static let timeoutSeconds: TimeInterval = 120

    private let log: (String, [String: Any]) -> Void

    init(log: @escaping (String, [String: Any]) -> Void) {
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
        let process = Process()
        process.executableURL = URL(fileURLWithPath: config.askhuman)
        process.arguments = ["todo-sync", "tick", "--reason", reason.rawValue]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        // launchd starts agents in `/`; a tick should not resolve anything relative to that.
        process.currentDirectoryURL = FileManager.default.homeDirectoryForCurrentUser
        var environment = ProcessInfo.processInfo.environment
        environment["ASKHUMAN_TODO_SYNC_CALLER"] = "helper"
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
