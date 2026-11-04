import Foundation
import Observation
import AHProtocol

// The "Add todo…" sheet of a request page (docs/specs/todo-quick-add.md §4.2 and §5.3): one sentence becomes
// a todo of the project the request belongs to, written to the Mac that sent the request. There is nothing to
// choose, so none of the quick-add's project list is here; the write is the same (`TodoAddCall`), and so are
// the reasons it cannot be made (`QuickTodoUnavailable`) and the ways it fails (`QuickTodoFailure`).

@MainActor
@Observable
public final class RequestTodoComposer {
    public enum Phase: Equatable, Sendable {
        case editing
        case sending
    }

    /// The Mac the request came from.
    public let macId: String
    /// The project as the request page names it, and the id the Mac knows it by: the hash of the project key
    /// that the Mac computes itself, so that it finds the project among the requests it is still waiting on.
    public let projectName: String
    public let projectId: String

    /// What is being written. Memory only, kept while the request is: closing the sheet does not lose it.
    public var text = ""
    public private(set) var phase: Phase = .editing
    public private(set) var failure: QuickTodoFailure?
    /// The Mac's name now (`nil` from the pairing: the name the request came with).
    public private(set) var macName: String

    /// The Mac as the app knows it; `nil` once it is not paired any more.
    private var mac: MacState?

    @ObservationIgnored private let backend: RemoteBackend
    /// The send whose answer got lost. Sending the same text again repeats it under the same id, which the Mac
    /// stores once, so a todo is never added twice.
    @ObservationIgnored private var lastAttempt: (clientMsgId: String, text: String)?

    public init(backend: RemoteBackend, macId: String, macName: String, projectKey: String, projectName: String, mac: MacState? = nil) {
        self.backend = backend
        self.macId = macId
        self.macName = mac?.name ?? macName
        self.mac = mac
        self.projectId = TodoProjectId.of(key: projectKey)
        let named = projectName.trimmingCharacters(in: .whitespacesAndNewlines)
        self.projectName = named.isEmpty ? (projectKey as NSString).lastPathComponent : named
    }

    /// The Macs have been heard from: what stands in the way of sending follows them.
    public func update(mac next: MacState?) {
        mac = next
        if let name = next?.name { macName = name }
    }

    /// What stands in the way of sending, in the order the sheet says it.
    public var unavailable: QuickTodoUnavailable? {
        guard let mac, mac.link == .online else { return .macOffline(mac: macName) }
        if !mac.caps.todos { return .macNeedsUpdate(mac: macName) }
        let count = text.trimmingCharacters(in: .whitespacesAndNewlines).count
        if count > Wire.todoTextMaxChars { return .tooLong(count: count, limit: Wire.todoTextMaxChars) }
        return nil
    }

    public var canSend: Bool {
        guard phase == .editing, unavailable == nil else { return false }
        if case .success = TodoText.validate(text) { return true }
        return false
    }

    /// Adds the text to the project, without ⚡ (the Mac's answer is waited for). Returns whether the Mac took
    /// it, and then the text is gone; otherwise the text stays and `failure` says why.
    @discardableResult
    public func send() async -> Bool {
        guard canSend, case .success(let body) = TodoText.validate(text) else { return false }
        phase = .sending
        failure = nil
        defer { phase = .editing }

        let clientMsgId = lastAttempt.flatMap { $0.text == body ? $0.clientMsgId : nil } ?? UUID().uuidString
        let outcome = await TodoAddCall.send(
            backend: backend, macId: macId, macName: macName, projectId: projectId, text: body, clientMsgId: clientMsgId
        )

        lastAttempt = nil
        switch outcome {
        case .added:
            text = ""
            return true
        case .projectGone:
            // The Mac does not know the project any more: there is no other project to suggest.
            failure = .refused("unknownProject")
        case .cancelled:
            break
        case .failed(let reason):
            // No answer came, twice: the same text sent again repeats the same write.
            if case .timedOut = reason { lastAttempt = (clientMsgId, body) }
            failure = reason
        }
        return false
    }
}
