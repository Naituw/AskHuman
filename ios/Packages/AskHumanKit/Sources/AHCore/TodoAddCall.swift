import Foundation
import AHProtocol

// The write both quick adds make (docs/specs/ios-developer-mode.md §6 and docs/specs/todo-quick-add.md §4.2):
// one `add` of a todo to a project of one Mac, without ⚡, whose answer is waited for. A lost answer is asked
// for again once under the same id, which the Mac stores once, so a todo is never added twice.

/// How asking a Mac to add one todo ended.
enum TodoAddOutcome: Equatable, Sendable {
    /// The Mac took it.
    case added
    /// The Mac does not know the project (any more).
    case projectGone
    /// It did not get through: nothing was sent, the Mac did not answer, or it refused or could not store it.
    case failed(QuickTodoFailure)
    /// The call was cancelled: there is nothing to say about it.
    case cancelled
}

enum TodoAddCall {
    /// Adds `text` to the Mac's project `projectId`. `macName` is only what a failure names.
    static func send(
        backend: RemoteBackend, macId: String, macName: String, projectId: String, text: String, clientMsgId: String
    ) async -> TodoAddOutcome {
        let operation = TodoMutation.add(localId: clientMsgId, text: text, auto: false)
            .operation(clientMsgId: clientMsgId, projectId: projectId)

        // A lost answer is asked for again once; the id makes the repeat harmless.
        var ack: TodoAck?
        var problem: Error?
        for _ in 0..<2 {
            do {
                ack = try await backend.todoApply(macId: macId, operation: operation)
                problem = nil
                break
            } catch BackendError.timedOut {
                problem = BackendError.timedOut
            } catch {
                problem = error
                break
            }
        }

        guard let ack else {
            switch problem as? BackendError {
            case .timedOut?: return .failed(.timedOut(mac: macName))
            case .macOffline?, .unknownMac?: return .failed(.unreachable(mac: macName))
            case .cancelled?: return .cancelled
            case .rejected(let reason)?, .unavailable(let reason)?, .invalid(let reason)?: return .failed(.refused(reason))
            default: return .failed(.notSaved)
            }
        }

        switch ack.status {
        case "ok": return .added
        case "notFound": return .projectGone
        case "rejected": return .failed(.refused(ack.reason ?? ""))
        default: return .failed(.notSaved)
        }
    }
}
