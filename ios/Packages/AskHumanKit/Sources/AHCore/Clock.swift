import Foundation

/// Time source of the session engine. Production uses the system clock; tests drive a manual one
/// so that timeouts (answer acknowledgements, reconnect back-off) run instantly and in order.
public protocol EngineClock: Sendable {
    func now() -> Date
    /// Sleeps for `seconds`; throws `CancellationError` when the task is cancelled.
    func sleep(seconds: TimeInterval) async throws
}

public extension EngineClock {
    func nowMs() -> Int64 { Int64(now().timeIntervalSince1970 * 1000) }
}

public struct SystemEngineClock: EngineClock {
    public init() {}
    public func now() -> Date { Date() }
    public func sleep(seconds: TimeInterval) async throws {
        try await Task.sleep(nanoseconds: UInt64(max(0, seconds) * 1_000_000_000))
    }
}

/// A clock that only moves when `advance(by:)` is called.
public final class ManualClock: EngineClock, @unchecked Sendable {
    private struct Sleeper {
        var wake: Date
        var continuation: CheckedContinuation<Void, Error>
    }

    private let lock = NSLock()
    private var current: Date
    private var sleepers: [UUID: Sleeper] = [:]

    public init(start: Date = Date(timeIntervalSince1970: 1_800_000_000)) { current = start }

    public func now() -> Date {
        lock.lock(); defer { lock.unlock() }
        return current
    }

    public func sleep(seconds: TimeInterval) async throws {
        let id = UUID()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
                register(id: id, seconds: seconds, continuation: cont)
            }
        } onCancel: {
            cancelSleeper(id)
        }
    }

    private func register(id: UUID, seconds: TimeInterval, continuation: CheckedContinuation<Void, Error>) {
        let outcome: Result<Void, Error>? = lock.withLock {
            if Task.isCancelled { return .failure(CancellationError()) }
            if seconds <= 0 { return .success(()) }
            sleepers[id] = Sleeper(wake: current.addingTimeInterval(seconds), continuation: continuation)
            return nil
        }
        if let outcome { continuation.resume(with: outcome) }
    }

    private func cancelSleeper(_ id: UUID) {
        let sleeper = lock.withLock { sleepers.removeValue(forKey: id) }
        sleeper?.continuation.resume(throwing: CancellationError())
    }

    /// Moves time forward, waking every sleeper whose time has come, earliest first.
    public func advance(by seconds: TimeInterval) async {
        let target = lock.withLock { current.addingTimeInterval(seconds) }
        while let sleeper = popSleeper(upTo: target) {
            sleeper.resume()
            // Let the woken task run (and register follow-up sleeps) before looking further.
            for _ in 0..<20 { await Task.yield() }
        }
        lock.withLock { current = max(current, target) }
    }

    private func popSleeper(upTo target: Date) -> CheckedContinuation<Void, Error>? {
        lock.withLock {
            guard let next = sleepers.filter({ $0.value.wake <= target }).min(by: { $0.value.wake < $1.value.wake }) else { return nil }
            sleepers[next.key] = nil
            current = max(current, next.value.wake)
            return next.value.continuation
        }
    }

    public var sleeperCount: Int {
        lock.lock(); defer { lock.unlock() }
        return sleepers.count
    }
}
