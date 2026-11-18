import Foundation

/// A checked continuation that takes the first result it is given and ignores the rest.
///
/// A callback API may call back twice for one request, and a second `resume` of a `CheckedContinuation` traps the whole
/// app. Photos does: cancelling a `loadTransferable` made its completion handler run from the cancellation after the
/// handler had resumed the continuation already, and the app crashed inside `resume`.
public final class OneShotContinuation<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Value, Error>?

    public init(_ continuation: CheckedContinuation<Value, Error>) {
        self.continuation = continuation
    }

    /// Delivers `result` unless an earlier call already delivered one. Returns whether this call was the one that did.
    @discardableResult
    public func resume(with result: Result<Value, Error>) -> Bool {
        lock.lock()
        let taken = continuation
        continuation = nil
        lock.unlock()
        guard let taken else { return false }
        taken.resume(with: result)
        return true
    }
}
