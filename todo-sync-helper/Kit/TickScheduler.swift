import Foundation

/// Decides WHEN the helper starts `AskHuman todo-sync tick` (plan P3 "触发器"). Pure: time is passed in,
/// so the rules are unit-tested without sleeping.
///
/// Rules:
///  * at most one tick runs at a time; triggers that arrive meanwhile are remembered and produce at most
///    one follow-up tick after it ends;
///  * every trigger has a debounce: a burst of file events becomes one tick (trailing edge), wake /
///    unlock / network events wait for the system to settle (leading edge, first one wins);
///  * a tick starts at least `minGapMs` after the previous one ended, so no event pattern can spin;
///  * when several reasons are due together the most informative one names the run
///    (`Reason.allCases` order).
final class TickScheduler {
    enum Reason: String, CaseIterable {
        case push
        case account
        case file
        case wake
        case unlock
        case network
        case poll
    }

    struct Timing: Equatable {
        /// Plan `LOCAL_DEBOUNCE_MS`.
        var fileDebounceMs: Int64 = 1_000
        /// Plan `WAKE_DEBOUNCE_MS`: wake, unlock and network-up arrive in bursts.
        var settleDebounceMs: Int64 = 2_000
        var pushDebounceMs: Int64 = 200
        var minGapMs: Int64 = 1_000
    }

    private let timing: Timing
    private var pending: [Reason: Int64] = [:]
    private(set) var running = false
    private var lastEndMs: Int64?

    init(timing: Timing = Timing()) {
        self.timing = timing
    }

    func trigger(_ reason: Reason, now: Int64) {
        let due = now + debounce(for: reason)
        switch reason {
        case .file:
            // Trailing edge: keep pushing the deadline while writes keep coming.
            pending[reason] = due
        default:
            pending[reason] = min(pending[reason] ?? due, due)
        }
    }

    /// If a tick should start now, marks it running and returns its reason. Everything pending is
    /// consumed: the tick reads the files and talks to the cloud after this moment, so it covers every
    /// trigger that arrived before it. Triggers that arrive while it runs are kept for a follow-up.
    func takeRun(now: Int64) -> Reason? {
        guard !running, !pending.isEmpty else { return nil }
        if let last = lastEndMs, now - last < timing.minGapMs { return nil }
        let due = Reason.allCases.filter { (pending[$0] ?? Int64.max) <= now }
        guard let reason = due.first else { return nil }
        pending.removeAll()
        running = true
        return reason
    }

    func finished(now: Int64) {
        running = false
        lastEndMs = now
    }

    /// When `takeRun` could next succeed (nil: nothing waiting, or a tick is running and will call back).
    func nextDue(now: Int64) -> Int64? {
        guard !running, let earliest = pending.values.min() else { return nil }
        if let last = lastEndMs { return max(earliest, last + timing.minGapMs) }
        return earliest
    }

    func clear() {
        pending.removeAll()
    }

    private func debounce(for reason: Reason) -> Int64 {
        switch reason {
        case .file: return timing.fileDebounceMs
        case .wake, .unlock, .network, .account: return timing.settleDebounceMs
        case .push: return timing.pushDebounceMs
        case .poll: return 0
        }
    }
}

/// Runs a [`TickScheduler`] on a serial queue: owns the timer, starts ticks through `runner`, and keeps
/// the bookkeeping `status` reports.
final class TickCoordinator {
    /// Starts one tick and calls `done` exactly once with its exit code (nil: could not be started).
    typealias Runner = (_ reason: TickScheduler.Reason, _ done: @escaping (Int32?) -> Void) -> Void

    struct Stats: Equatable {
        var ticksRun = 0
        var lastTickMs: Int64 = 0
        var lastReason: String?
        var lastExit: Int?
    }

    private let queue: DispatchQueue
    private let scheduler: TickScheduler
    private let clock: () -> Int64
    private let runner: Runner
    private let log: (String, [String: Any]) -> Void
    private var timer: DispatchSourceTimer?
    private var enabled = true
    private var stats = Stats()

    init(
        timing: TickScheduler.Timing = TickScheduler.Timing(),
        clock: @escaping () -> Int64 = { Int64(Date().timeIntervalSince1970 * 1000) },
        queue: DispatchQueue = DispatchQueue(label: "todosync.tick-coordinator"),
        log: @escaping (String, [String: Any]) -> Void = { _, _ in },
        runner: @escaping Runner
    ) {
        self.scheduler = TickScheduler(timing: timing)
        self.clock = clock
        self.queue = queue
        self.log = log
        self.runner = runner
    }

    func trigger(_ reason: TickScheduler.Reason) {
        queue.async {
            guard self.enabled else { return }
            self.scheduler.trigger(reason, now: self.clock())
            self.pump()
        }
    }

    /// Off: pending triggers are dropped and new ones ignored (sync turned off in the core).
    func setEnabled(_ on: Bool) {
        queue.async {
            self.enabled = on
            if !on {
                self.scheduler.clear()
                self.timer?.cancel()
                self.timer = nil
            }
        }
    }

    func snapshot() -> Stats {
        queue.sync { stats }
    }

    private func pump() {
        timer?.cancel()
        timer = nil
        let now = clock()
        if let reason = scheduler.takeRun(now: now) {
            let startedAt = now
            runner(reason) { [weak self] exit in
                guard let self else { return }
                self.queue.async {
                    let end = self.clock()
                    self.scheduler.finished(now: end)
                    self.stats.ticksRun += 1
                    self.stats.lastTickMs = end
                    self.stats.lastReason = reason.rawValue
                    self.stats.lastExit = exit.map(Int.init)
                    self.log("tick_done", ["reason": reason.rawValue, "exit": exit.map(Int.init) ?? -1, "ms": end - startedAt])
                    self.pump()
                }
            }
            return
        }
        guard enabled, let due = scheduler.nextDue(now: now) else { return }
        let delayMs = max(due - now, 1)
        let source = DispatchSource.makeTimerSource(queue: queue)
        source.schedule(deadline: .now() + .milliseconds(Int(delayMs)), leeway: .milliseconds(20))
        source.setEventHandler { [weak self] in self?.pump() }
        timer = source
        source.resume()
    }
}
