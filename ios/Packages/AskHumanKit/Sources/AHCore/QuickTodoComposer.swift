import Foundation
import Observation
import AHProtocol

// The quick-add page of the developer mode (docs/specs/ios-developer-mode.md §6): one sentence becomes a
// todo of a remembered project, from wherever the phone is. It writes through the same `todo.op` as the
// Todos tab and keeps nothing of the text on disk (docs/specs/ios-todos.md I13: no offline queue).

/// Where quick-added todos go: a project on a Mac. What is stored on the phone is only names and the
/// Mac's opaque ids, never a path.
public struct QuickTodoTarget: Codable, Equatable, Hashable, Sendable {
    public var macId: String
    public var macName: String
    public var projectId: String
    public var projectName: String
    /// The suffix that tells projects of the same name apart ("work", "playground"), when there is one.
    public var projectHint: String?

    public init(macId: String, macName: String, projectId: String, projectName: String, projectHint: String? = nil) {
        self.macId = macId
        self.macName = macName
        self.projectId = projectId
        self.projectName = projectName
        self.projectHint = projectHint
    }
}

/// Why "Add" cannot be tapped (the empty text needs no explanation and is not among them).
public enum QuickTodoUnavailable: Equatable, Sendable {
    case noProject
    case macOffline(mac: String)
    case macNeedsUpdate(mac: String)
    case tooLong(count: Int, limit: Int)
}

/// Why the last send did not make it.
public enum QuickTodoFailure: Equatable, Sendable {
    /// The Mac could not be reached, so nothing was sent.
    case unreachable(mac: String)
    /// No answer came, twice: the todo may or may not be there.
    case timedOut(mac: String)
    /// The Mac does not know the project any more.
    case projectGone(mac: String)
    /// The Mac could not store it.
    case notSaved
    /// The Mac refused: `empty`, `tooLong`, `unsupported`, …
    case refused(String)
}

/// One Mac in the list the project is chosen from.
public struct QuickTodoGroup: Identifiable, Equatable, Sendable {
    public enum State: Equatable, Sendable {
        case loading
        case projects([TodoListedProject])
        case offline
        case needsUpdate
        /// The Mac was asked and did not answer.
        case noAnswer
    }

    public var macId: String
    public var macName: String
    public var state: State

    public var id: String { macId }
}

@MainActor
@Observable
public final class QuickTodoComposer {
    public enum Phase: Equatable, Sendable {
        case editing
        case sending
    }

    /// What is being written. Memory only: it survives closing the page, not the app.
    public var text = ""
    /// The search of the project list.
    public var query = ""
    public private(set) var phase: Phase = .editing
    public private(set) var failure: QuickTodoFailure?
    /// The Macs and what they have, for the project list.
    public private(set) var groups: [QuickTodoGroup] = []

    private var stored: QuickTodoTarget?
    private var macs: [MacState] = []

    @ObservationIgnored private let backend: RemoteBackend
    @ObservationIgnored private let defaults: UserDefaults
    /// Bumped whenever the project list is asked for anew or closed: answers of an older ask are dropped.
    @ObservationIgnored private var epoch = 0
    @ObservationIgnored private var picking = false
    /// The send whose answer got lost. Sending the same text to the same project again repeats it under the
    /// same id, which the Mac stores once, so a todo is never added twice.
    @ObservationIgnored private var lastAttempt: Attempt?

    private struct Attempt {
        var clientMsgId: String
        var text: String
        var target: QuickTodoTarget
    }

    private static let targetKey = "ah.dev.quickTodoTarget"

    public init(backend: RemoteBackend, defaults: UserDefaults = .standard) {
        self.backend = backend
        self.defaults = defaults
        stored = defaults.data(forKey: Self.targetKey).flatMap { try? JSONDecoder().decode(QuickTodoTarget.self, from: $0) }
    }

    // MARK: the target

    /// The project the todo goes to, with the Mac's name as it is now. A project of a Mac that is not paired
    /// any more counts as none (it is kept, and comes back if that Mac is paired again).
    public var target: QuickTodoTarget? {
        guard var target = stored, let mac = macs.first(where: { $0.id == target.macId }) else { return nil }
        target.macName = mac.name
        return target
    }

    public func choose(_ project: TodoListedProject) {
        stored = QuickTodoTarget(
            macId: project.macId, macName: project.macName, projectId: project.id, projectName: project.name,
            projectHint: project.hint
        )
        failure = nil
        lastAttempt = nil
        persistTarget()
    }

    private func persistTarget() {
        if let stored, let data = try? JSONEncoder().encode(stored) {
            defaults.set(data, forKey: Self.targetKey)
        } else {
            defaults.removeObject(forKey: Self.targetKey)
        }
    }

    // MARK: whether it can be sent

    private var targetMac: MacState? {
        target.flatMap { target in macs.first { $0.id == target.macId } }
    }

    /// What stands in the way of sending, in the order the page says it.
    public var unavailable: QuickTodoUnavailable? {
        guard let target, let mac = targetMac else { return .noProject }
        if mac.link != .online { return .macOffline(mac: target.macName) }
        if !mac.caps.todos { return .macNeedsUpdate(mac: target.macName) }
        let count = text.trimmingCharacters(in: .whitespacesAndNewlines).count
        if count > Wire.todoTextMaxChars { return .tooLong(count: count, limit: Wire.todoTextMaxChars) }
        return nil
    }

    public var canSend: Bool {
        guard phase == .editing, unavailable == nil else { return false }
        if case .success = TodoText.validate(text) { return true }
        return false
    }

    // MARK: sending

    /// Adds the text to the target project, without ⚡ (the Mac's answer is waited for). Returns the target when
    /// the Mac took it, and then the text is gone; otherwise `nil`, the text stays and `failure` says why.
    @discardableResult
    public func send() async -> QuickTodoTarget? {
        guard canSend, let target, case .success(let body) = TodoText.validate(text) else { return nil }
        phase = .sending
        failure = nil
        defer { phase = .editing }

        let clientMsgId = lastAttempt.flatMap { $0.target == target && $0.text == body ? $0.clientMsgId : nil } ?? UUID().uuidString
        let operation = TodoMutation.add(localId: clientMsgId, text: body, auto: false)
            .operation(clientMsgId: clientMsgId, projectId: target.projectId)

        // A lost answer is asked for again once; the id makes the repeat harmless.
        var ack: TodoAck?
        var problem: Error?
        for _ in 0..<2 {
            do {
                ack = try await backend.todoApply(macId: target.macId, operation: operation)
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
            lastAttempt = nil
            switch problem as? BackendError {
            case .timedOut?:
                lastAttempt = Attempt(clientMsgId: clientMsgId, text: body, target: target)
                failure = .timedOut(mac: target.macName)
            case .macOffline?, .unknownMac?:
                failure = .unreachable(mac: target.macName)
            case .cancelled?:
                break
            case .rejected(let reason)?, .unavailable(let reason)?, .invalid(let reason)?:
                failure = .refused(reason)
            default:
                failure = .notSaved
            }
            return nil
        }

        lastAttempt = nil
        switch ack.status {
        case "ok":
            text = ""
            return target
        case "notFound":
            // The project is gone from that Mac: it cannot stay the target.
            stored = nil
            persistTarget()
            failure = .projectGone(mac: target.macName)
        case "rejected":
            failure = .refused(ack.reason ?? "")
        default:
            failure = .notSaved
        }
        return nil
    }

    // MARK: the project list

    /// The Macs have been heard from (or changed): a list that is open follows them.
    public func update(macs next: [MacState]) {
        let before = Self.usable(macs)
        macs = next
        if picking, Self.usable(next) != before { Task { await loadProjects() } }
    }

    private static func usable(_ list: [MacState]) -> Set<String> {
        Set(list.filter { $0.link == .online && $0.caps.todos }.map(\.id))
    }

    /// Asks every Mac that is online and knows todos for its projects, all at once; the others are listed
    /// with the reason they have none. Rows come in the order the Mac gave them.
    public func loadProjects() async {
        picking = true
        epoch += 1
        let mine = epoch
        groups = macs.map { mac in
            let state: QuickTodoGroup.State =
                mac.link != .online ? .offline : (mac.caps.todos ? .loading : .needsUpdate)
            return QuickTodoGroup(macId: mac.id, macName: mac.name, state: state)
        }
        let asked = macs.filter { $0.link == .online && $0.caps.todos }.map(\.id)
        let tasks = asked.map { id in
            Task { @MainActor in
                let result = await self.fetch(id)
                self.adopt(result, from: id, epoch: mine)
            }
        }
        for task in tasks { await task.value }
    }

    /// Asks one Mac again, after it did not answer.
    public func retry(macId: String) async {
        guard let index = groups.firstIndex(where: { $0.macId == macId }) else { return }
        groups[index].state = .loading
        let mine = epoch
        adopt(await fetch(macId), from: macId, epoch: mine)
    }

    /// The list is not on screen any more: answers still on their way are dropped.
    public func closeProjects() {
        picking = false
        epoch += 1
    }

    /// The groups the search leaves. While searching, only projects are shown, and only the Macs that have a match.
    public var visibleGroups: [QuickTodoGroup] {
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { return groups }
        return groups.compactMap { group in
            guard case .projects(let list) = group.state else { return nil }
            let matched = list.filter { TodoRows.matches($0, query: query) }
            return matched.isEmpty ? nil : QuickTodoGroup(macId: group.macId, macName: group.macName, state: .projects(matched))
        }
    }

    private func fetch(_ macId: String) async -> Result<TodoProjects, Error> {
        do { return .success(try await backend.todoProjects(macId: macId)) } catch { return .failure(error) }
    }

    private func adopt(_ result: Result<TodoProjects, Error>, from macId: String, epoch mine: Int) {
        guard mine == epoch, let index = groups.firstIndex(where: { $0.macId == macId }) else { return }
        let name = groups[index].macName
        switch result {
        case .success(let reply):
            groups[index].state = .projects(reply.projects.map { TodoListedProject(macId: macId, macName: name, row: $0) })
        case .failure(let error):
            switch error as? BackendError {
            case .macOffline?, .unknownMac?: groups[index].state = .offline
            case .rejected("unsupported")?: groups[index].state = .needsUpdate
            default: groups[index].state = .noAnswer
            }
        }
    }
}
