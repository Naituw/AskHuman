import XCTest

/// What the asset tests share (`AssetPreviewTests`, `AssetContentCacheTests`).

struct StillWaiting: Error {}

/// Resumes a continuation once, whichever of two racers gets there first.
final class Once<T: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    private var spent = false

    func resume(_ continuation: CheckedContinuation<T, Error>, with result: Result<T, Error>) {
        let first = lock.withLock { () -> Bool in
            defer { spent = true }
            return !spent
        }
        if first { continuation.resume(with: result) }
    }
}

extension XCTestCase {
    /// The result of `task`. A download that never finishes fails its test after a few seconds of real time instead of
    /// hanging the run (the engine's own timers run on a manual clock that nothing advances here): the task is
    /// cancelled, and the test is told it is still waiting if even that does not end it.
    func finish<T: Sendable>(_ task: Task<T, Error>, within seconds: Double = 5) async throws -> T {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<T, Error>) in
            let once = Once<T>()
            Task { once.resume(continuation, with: await task.result) }
            Task {
                try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
                task.cancel()
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                once.resume(continuation, with: .failure(StillWaiting()))
            }
        }
    }
}
