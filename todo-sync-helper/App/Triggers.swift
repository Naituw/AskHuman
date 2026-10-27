import AppKit
import Foundation
import Network

/// Everything that can make the helper start a tick, translated into [`TickScheduler.Reason`]s:
/// the poll timer, system wake and unlock, the network coming back, edits of `todos.json`, and (from the
/// app delegate) CloudKit pushes and iCloud account changes.
final class Triggers {
    private let coordinator: TickCoordinator
    private let log: (String, [String: Any]) -> Void

    private var pollTimer: DispatchSourceTimer?
    private var pollMinutes = 0
    private var fileWatcher: FileChangeWatcher?
    private var watchedPath: String?
    private var pathMonitor: NWPathMonitor?
    private var observers: [(NotificationCenter, NSObjectProtocol)] = []
    private let queue = DispatchQueue(label: "todosync.triggers")

    init(coordinator: TickCoordinator, log: @escaping (String, [String: Any]) -> Void) {
        self.coordinator = coordinator
        self.log = log
    }

    /// System-level sources never change: install them once.
    func startSystemSources() {
        let workspace = NSWorkspace.shared.notificationCenter
        observe(workspace, NSWorkspace.didWakeNotification, .wake)
        observe(workspace, NSWorkspace.sessionDidBecomeActiveNotification, .unlock)
        observe(DistributedNotificationCenter.default(), Notification.Name("com.apple.screenIsUnlocked"), .unlock)

        let monitor = NWPathMonitor()
        var wasSatisfied: Bool?
        monitor.pathUpdateHandler = { [weak self] path in
            let satisfied = path.status == .satisfied
            // Only "came back" matters; the first callback just reports the current state.
            if satisfied, wasSatisfied == false { self?.coordinator.trigger(.network) }
            wasSatisfied = satisfied
        }
        monitor.start(queue: queue)
        pathMonitor = monitor
    }

    /// Config-dependent sources: restart them when the config changed.
    func apply(config: HelperConfig?) {
        queue.async { [self] in
            let minutes = (config?.enabled == true) ? config!.pollMinutes : 0
            if minutes != pollMinutes {
                pollTimer?.cancel()
                pollTimer = nil
                pollMinutes = minutes
                if minutes > 0 {
                    let interval = DispatchTimeInterval.seconds(minutes * 60)
                    let timer = DispatchSource.makeTimerSource(queue: queue)
                    timer.schedule(deadline: .now() + interval, repeating: interval, leeway: .seconds(30))
                    timer.setEventHandler { [weak self] in self?.coordinator.trigger(.poll) }
                    timer.resume()
                    pollTimer = timer
                }
                log("poll_timer", ["minutes": minutes])
            }

            let path = (config?.enabled == true) ? config?.todosPath : nil
            if path != watchedPath {
                fileWatcher?.stop()
                fileWatcher = nil
                watchedPath = path
                if let path {
                    let watcher = FileChangeWatcher(fileURL: URL(fileURLWithPath: path)) { [weak self] in
                        self?.coordinator.trigger(.file)
                    }
                    if !watcher.start() { log("file_watch_unavailable", [:]) }
                    fileWatcher = watcher
                }
            }
        }
    }

    func stop() {
        queue.sync {
            pollTimer?.cancel()
            pollTimer = nil
            fileWatcher?.stop()
            fileWatcher = nil
            pathMonitor?.cancel()
            pathMonitor = nil
        }
        for (center, token) in observers { center.removeObserver(token) }
        observers.removeAll()
    }

    private func observe(_ center: NotificationCenter, _ name: Notification.Name, _ reason: TickScheduler.Reason) {
        let token = center.addObserver(forName: name, object: nil, queue: nil) { [weak self] _ in
            self?.coordinator.trigger(reason)
        }
        observers.append((center, token))
    }
}
