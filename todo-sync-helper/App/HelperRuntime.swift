import AppKit
import CloudKit
import Foundation

/// Wires the helper together: backend, RPC server, tick coordinator, triggers and the config hand-over
/// file. One instance for the life of the process.
final class HelperRuntime: StatusSource {
    let paths: HelperPaths
    private let startedAt = Date()
    private let log: (String, [String: Any]) -> Void
    private let stateLock = NSLock()

    private var instanceLock: InstanceLock?
    private var server: RpcSocketServer?
    private var backend: CloudBackend
    private var coordinator: TickCoordinator!
    private var triggers: Triggers!
    private var configWatcher: FileChangeWatcher?
    private var config: HelperConfig?
    private var apns = "pending"
    private var lastPushMs: Int64 = 0
    private var activity: NSObjectProtocol?
    private var accountObserver: NSObjectProtocol?

    static var helperVersion: String { CommandLineTool.version }

    init(paths: HelperPaths = .resolve()) {
        self.paths = paths
        let logFn: (String, [String: Any]) -> Void = { HelperLog.shared.write($0, $1) }
        self.log = logFn

        let container = (Bundle.main.object(forInfoDictionaryKey: "HelperContainerID") as? String) ?? ""
        let environment = Entitlements.cloudKitEnvironment()
        if container.isEmpty {
            backend = UnavailableBackend(environment: environment, container: container, problem: "noContainer")
        } else if !Entitlements.grantsCloudKit(container: container) {
            // `CKContainer(identifier:)` would trap without the entitlement.
            backend = UnavailableBackend(environment: environment, container: container, problem: "missingEntitlement")
        } else {
            backend = CloudKitBackend(containerID: container, environment: environment)
        }
    }

    /// False when another helper already owns this state directory.
    func start() -> Bool {
        guard let lock = InstanceLock.acquire(at: paths.lockURL) else {
            log("already_running", [:])
            return false
        }
        instanceLock = lock
        activity = ProcessInfo.processInfo.beginActivity(
            options: [.userInitiatedAllowingIdleSystemSleep], reason: "todo sync helper")

        let runner = TickProcessRunner(log: log)
        coordinator = TickCoordinator(log: log) { [weak self] reason, done in
            guard let self else { return done(nil) }
            runner.run(config: self.currentConfig(), reason: reason, done: done)
        }
        triggers = Triggers(coordinator: coordinator, log: log)
        triggers.startSystemSources()

        let dispatcher = RpcDispatcher(backend: backend, helperVersion: Self.helperVersion, statusSource: self, log: log)
        let server = RpcSocketServer(path: paths.socketURL.path, log: log) { await dispatcher.handle($0) }
        do {
            try server.start()
            self.server = server
        } catch {
            log("server_start_failed", ["error": "\(error)"])
            return false
        }

        reloadConfig()
        let watcher = FileChangeWatcher(fileURL: paths.configURL) { [weak self] in self?.reloadConfig() }
        if !watcher.start() { log("config_watch_unavailable", [:]) }
        configWatcher = watcher

        if let cloud = backend as? CloudKitBackend {
            accountObserver = NotificationCenter.default.addObserver(forName: .CKAccountChanged, object: nil, queue: nil) { [weak self] _ in
                cloud.accountChanged()
                self?.log("account_changed", [:])
                self?.coordinator.trigger(.account)
            }
        }

        log("started", [
            "version": Self.helperVersion, "environment": Entitlements.cloudKitEnvironment(), "pid": Int(getpid()),
            "push": Entitlements.hasPushEntitlement, "backend": backend is CloudKitBackend ? "cloudkit" : "unavailable",
        ])
        // Catch up on whatever happened while the helper was not running.
        coordinator.trigger(.wake)
        return true
    }

    func stop() {
        configWatcher?.stop()
        triggers?.stop()
        server?.stop()
        instanceLock?.release()
        if let activity { ProcessInfo.processInfo.endActivity(activity) }
        log("stopped", [:])
        HelperLog.shared.flush()
    }

    // MARK: - Config

    private func currentConfig() -> HelperConfig? {
        stateLock.lock()
        defer { stateLock.unlock() }
        return config
    }

    private func reloadConfig() {
        let loaded = HelperConfig.load(from: paths.configURL)
        stateLock.lock()
        let changed = loaded != config
        config = loaded
        stateLock.unlock()
        guard changed else { return }
        log("config_loaded", [
            "present": loaded != nil, "enabled": loaded?.enabled ?? false, "pollMinutes": loaded?.pollMinutes ?? 0,
        ])
        coordinator.setEnabled(loaded?.enabled == true)
        triggers.apply(config: loaded)
        // A freshly enabled core wants its first sync now, not at the next poll.
        if loaded?.enabled == true { coordinator.trigger(.wake) }
    }

    // MARK: - Push

    func apnsChanged(_ state: String) {
        stateLock.lock()
        apns = state
        stateLock.unlock()
        log("apns", ["state": state])
    }

    func pushReceived(subscriptionID: String?) {
        stateLock.lock()
        lastPushMs = Clock.nowMs()
        stateLock.unlock()
        log("push", ["ours": subscriptionID == CloudKitBackend.subscriptionID])
        coordinator.trigger(.push)
    }

    // MARK: - StatusSource

    func statusExtras() -> StatusExtras {
        stateLock.lock()
        let apnsState = apns
        let pushMs = lastPushMs
        let cfg = config
        stateLock.unlock()
        let stats = coordinator.snapshot()
        return StatusExtras(
            apns: Entitlements.hasPushEntitlement ? apnsState : "unsupported",
            lastPushMs: pushMs, lastTickMs: stats.lastTickMs, lastTickReason: stats.lastReason,
            lastTickExit: stats.lastExit, ticksRun: stats.ticksRun, enabled: cfg?.enabled ?? false,
            pollMinutes: cfg?.pollMinutes ?? HelperConfig.defaultPollMinutes, pid: getpid(),
            uptimeMs: Int64(Date().timeIntervalSince(startedAt) * 1000))
    }
}
