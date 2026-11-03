import Foundation
import Observation
import AHProtocol

// The quick-add page of the developer mode (docs/specs/ios-developer-mode.md §6): one sentence becomes a
// todo of a remembered project, from wherever the phone is. It writes through the same `todo.op` as the
// Todos tab and keeps nothing of the text on disk (docs/specs/ios-todos.md I13: no offline queue).

/// Where quick-added todos go: a project on one Mac, or an iCloud project that any Mac with sync can take.
/// What is stored on the phone is only names, the Mac's opaque ids and a repository address, never a path.
public struct QuickTodoTarget: Codable, Equatable, Hashable, Sendable {
    /// The Mac that listed the project; for an iCloud project, the Mac that served iCloud when it was chosen.
    public var macId: String
    public var macName: String
    public var projectId: String
    public var projectName: String
    /// The suffix that tells projects of the same name apart ("work", "playground"), when there is one.
    public var projectHint: String?
    /// An iCloud project: its repository, which names it on every Mac that syncs (each of them has its own id
    /// for it). The todo goes through `macId` while that Mac can take it, and through another Mac behind iCloud
    /// when it cannot. `nil` for a project that is on one Mac only.
    public var icloudRepo: String?

    public var viaICloud: Bool { icloudRepo != nil }

    public init(
        macId: String, macName: String, projectId: String, projectName: String, projectHint: String? = nil,
        icloudRepo: String? = nil
    ) {
        self.macId = macId
        self.macName = macName
        self.projectId = projectId
        self.projectName = projectName
        self.projectHint = projectHint
        self.icloudRepo = icloudRepo
    }
}

/// Why "Add" cannot be tapped (the empty text needs no explanation and is not among them).
public enum QuickTodoUnavailable: Equatable, Sendable {
    case noProject
    case macOffline(mac: String)
    case macNeedsUpdate(mac: String)
    /// An iCloud project, and no Mac that syncs can be reached.
    case icloudOffline
    case tooLong(count: Int, limit: Int)
}

/// Why the last send did not make it.
public enum QuickTodoFailure: Equatable, Sendable {
    /// The Mac could not be reached, so nothing was sent.
    case unreachable(mac: String)
    /// An iCloud project, and no Mac that syncs could be reached (or none of them knows the project).
    case icloudUnreachable
    /// No answer came, twice: the todo may or may not be there.
    case timedOut(mac: String)
    /// The Mac does not know the project any more (`nil`: iCloud's project).
    case projectGone(mac: String?)
    /// The Mac could not store it.
    case notSaved
    /// The Mac refused: `empty`, `tooLong`, `unsupported`, …
    case refused(String)
}

/// One entry of the first list the project is chosen from: where to look, as the title of the Todos tab offers
/// it. iCloud stands for every Mac that syncs; a Mac that does not is a source of its own.
public struct QuickTodoSource: Identifiable, Equatable, Sendable {
    public enum State: Equatable, Sendable {
        case loading
        /// The rows of the source, each with the Mac that told it.
        case projects([TodoListedProject])
        /// It could not be listed; `TodoFailure.timedOut` and `.other` are worth asking again.
        case failed(TodoFailure)
    }

    public var choice: TodoSourceChoice
    public var state: State
    /// iCloud: the name of the Mac whose answer the rows are.
    public var servedBy: String?

    public init(choice: TodoSourceChoice, state: State, servedBy: String? = nil) {
        self.choice = choice
        self.state = state
        self.servedBy = servedBy
    }

    public var id: String { choice.id }
    public var source: TodoSource { choice.source }

    /// Whether there is anything to look at behind the row: not for a Mac that is out of reach or too old, nor
    /// for iCloud while no Mac that syncs can be reached.
    public var canOpen: Bool {
        switch choice.availability {
        case .offline, .needsUpdate, .noSync: return false
        case .available: break
        }
        if case .failed(let failure) = state {
            switch failure {
            case .timedOut, .other: return true
            case .noMacs, .offline, .needsUpdate, .noSyncMac: return false
            }
        }
        return true
    }
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
    /// The "projects without todos" section of the list is open (it stays shut until asked for, as on the Todos tab).
    public var emptyExpanded = false
    public private(set) var phase: Phase = .editing
    public private(set) var failure: QuickTodoFailure?

    private var stored: QuickTodoTarget?
    private var macs: [MacState] = []

    /// What each Mac answered when it was asked for its projects. Kept until the list is asked for anew.
    private var answers: [String: Answer] = [:]
    /// What the Todos tab last heard each Mac do with its todos, and what the Macs said when this list asked
    /// them (that wins; a Mac that goes offline keeps what it was last heard to do, as on the Todos tab).
    private var storedModes: [String: TodoMode] = [:]
    private var learnedModes: [String: TodoMode] = [:]
    /// The Mac that served iCloud last on the Todos tab.
    private var lastICloudMac: String?

    private enum Answer {
        case pending
        case projects(TodoProjects)
        case failed(TodoFailure)
    }

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
        storedModes = TodosStore.storedModes(defaults)
        lastICloudMac = defaults.string(forKey: TodosStore.icloudMacKey)
    }

    // MARK: the target

    /// The project the todo goes to, with the Mac's name as it is now. A project of a Mac that is not paired
    /// any more counts as none (it is kept, and comes back if that Mac is paired again). An iCloud project is
    /// not tied to the Mac it was chosen through: it stays while any Mac is paired.
    public var target: QuickTodoTarget? {
        guard var target = stored else { return nil }
        if let mac = macs.first(where: { $0.id == target.macId }) {
            target.macName = mac.name
            return target
        }
        return target.viaICloud && !macs.isEmpty ? target : nil
    }

    /// The source the target is listed under: iCloud for an iCloud project and for a project that only a Mac
    /// behind iCloud has, else the Mac itself.
    public var currentSource: TodoSource? {
        guard let target else { return nil }
        if target.viaICloud { return .icloud }
        return TodoSources.isAbsorbed(target.macId, macs: macs, modes: modes) ? .icloud : .mac(target.macId)
    }

    /// Whether `project`, as `source` lists it, is the one the todos go to now.
    public func isCurrent(_ project: TodoListedProject, in source: TodoSource) -> Bool {
        guard let stored else { return false }
        if let repo = stored.icloudRepo { return source == .icloud && project.repo == repo }
        return stored.macId == project.macId && stored.projectId == project.id
    }

    /// Makes `project`, as `source` lists it, the target. An iCloud project (one with a repository) is chosen
    /// as iCloud's, not as the Mac's that happened to serve it; one that exists on a single Mac stays that Mac's.
    public func choose(_ project: TodoListedProject, in source: TodoSource) {
        let repo = source == .icloud && !project.isOnlyOnItsMac ? project.repo : nil
        stored = QuickTodoTarget(
            macId: project.macId, macName: project.macName, projectId: project.id, projectName: project.name,
            projectHint: project.hint, icloudRepo: repo
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

    private var modes: [String: TodoMode] {
        storedModes.merging(learnedModes) { _, learned in learned }
    }

    /// The Macs that can serve iCloud now, best first (the one the project was chosen through first).
    private func icloudCandidates(lastUsed: String?) -> [String] {
        TodoSources.icloudCandidates(macs: macs, modes: modes, lastUsed: lastUsed, failed: [])
    }

    /// What stands in the way of sending, in the order the page says it.
    public var unavailable: QuickTodoUnavailable? {
        guard let target else { return .noProject }
        if target.viaICloud {
            if icloudCandidates(lastUsed: target.macId).isEmpty { return .icloudOffline }
        } else {
            guard let mac = targetMac else { return .noProject }
            if mac.link != .online { return .macOffline(mac: target.macName) }
            if !mac.caps.todos { return .macNeedsUpdate(mac: target.macName) }
        }
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

    /// Where a send goes: a Mac, and that Mac's id for the project.
    private struct Route {
        var macId: String
        var macName: String
        var projectId: String
    }

    /// A project on one Mac goes to that Mac. An iCloud project goes through the Mac it was chosen through
    /// while that one can serve iCloud, and otherwise through the first other Mac that can and knows it: that
    /// Mac has its own id for the project, found by the repository.
    private func route(for target: QuickTodoTarget) async -> Route? {
        func name(_ id: String) -> String { macs.first { $0.id == id }?.name ?? target.macName }
        guard let repo = target.icloudRepo else {
            return Route(macId: target.macId, macName: target.macName, projectId: target.projectId)
        }
        let candidates = icloudCandidates(lastUsed: target.macId)
        if candidates.contains(target.macId) {
            return Route(macId: target.macId, macName: name(target.macId), projectId: target.projectId)
        }
        for id in candidates {
            guard let reply = try? await backend.todoProjects(macId: id), TodoMode(rawValue: reply.mode) == .icloud,
                let row = reply.projects.first(where: { $0.repo == repo })
            else { continue }
            return Route(macId: id, macName: name(id), projectId: row.id)
        }
        return nil
    }

    /// Adds the text to the target project, without ⚡ (the Mac's answer is waited for). Returns the target when
    /// the Mac took it, and then the text is gone; otherwise `nil`, the text stays and `failure` says why.
    @discardableResult
    public func send() async -> QuickTodoTarget? {
        guard canSend, let target, case .success(let body) = TodoText.validate(text) else { return nil }
        phase = .sending
        failure = nil
        defer { phase = .editing }

        // What the Todos tab has heard since is what the Macs are taken to do.
        storedModes = TodosStore.storedModes(defaults)
        guard let route = await route(for: target) else {
            lastAttempt = nil
            failure = .icloudUnreachable
            return nil
        }

        let clientMsgId = lastAttempt.flatMap { $0.target == target && $0.text == body ? $0.clientMsgId : nil } ?? UUID().uuidString
        let operation = TodoMutation.add(localId: clientMsgId, text: body, auto: false)
            .operation(clientMsgId: clientMsgId, projectId: route.projectId)

        // A lost answer is asked for again once; the id makes the repeat harmless.
        var ack: TodoAck?
        var problem: Error?
        for _ in 0..<2 {
            do {
                ack = try await backend.todoApply(macId: route.macId, operation: operation)
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
                failure = .timedOut(mac: route.macName)
            case .macOffline?, .unknownMac?:
                failure = .unreachable(mac: route.macName)
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
            failure = .projectGone(mac: target.viaICloud ? nil : target.macName)
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
        // What was heard of a Mac that is not paired any more goes.
        let paired = Set(next.map(\.id))
        if !next.isEmpty, learnedModes.keys.contains(where: { !paired.contains($0) }) {
            learnedModes = learnedModes.filter { paired.contains($0.key) }
        }
        if picking, Self.usable(next) != before { startLoadingProjects() }
    }

    private static func usable(_ list: [MacState]) -> Set<String> {
        Set(list.filter { $0.link == .online && $0.caps.todos }.map(\.id))
    }

    /// The sources to choose from, as the Todos tab's menu lists them (iCloud first; a Mac that syncs is part
    /// of it, with several Macs paired), each with what it holds so far.
    public var sources: [QuickTodoSource] {
        TodoSources.choices(macs: macs, modes: modes).map { choice in
            switch choice.source {
            case .icloud: return icloud(choice)
            case .mac(let id): return QuickTodoSource(choice: choice, state: state(ofMac: id))
            }
        }
    }

    /// Some Mac that was asked has not answered yet, and nothing is known of what it does with its todos: the
    /// sources would change as soon as it does (a Mac that syncs becomes part of iCloud). The list of them waits.
    public var isLearning: Bool {
        macs.contains { mac in
            guard Self.usable([mac]).contains(mac.id), modes[mac.id] == nil else { return false }
            switch answers[mac.id] {
            case nil, .pending?: return true
            case .projects?, .failed?: return false
            }
        }
    }

    /// The groups of the projects of `source` under the search, as the Todos tab groups them.
    public func sections(of source: TodoSource) -> [TodoSection] {
        guard case .projects(let rows)? = sources.first(where: { $0.source == source })?.state else { return [] }
        return TodoRows.sections(listed: rows, source: source, query: query, emptyExpanded: emptyExpanded)
    }

    /// A paired Mac's name.
    public func macName(_ id: String?) -> String? {
        id.flatMap { id in macs.first { $0.id == id }?.name }
    }

    private func state(ofMac id: String) -> QuickTodoSource.State {
        guard let mac = macs.first(where: { $0.id == id }) else { return .failed(.offline) }
        if !mac.caps.todos { return .failed(.needsUpdate) }
        if mac.link != .online { return .failed(.offline) }
        switch answers[id] {
        case nil, .pending?:
            return .loading
        case .projects(let reply)?:
            return .projects(TodoRows.listed(reply, of: .mac(id), servedBy: id, macs: macs, others: [:]))
        case .failed(let failure)?:
            return .failed(failure)
        }
    }

    /// iCloud is served by the first Mac in the ranking that answered with sync on, like the Todos tab picks
    /// its Mac; what the other Macs behind it have of their own is listed with it.
    private func icloud(_ choice: TodoSourceChoice) -> QuickTodoSource {
        var lastFailure: TodoFailure?
        for id in icloudCandidates(lastUsed: lastICloudMac) {
            switch answers[id] {
            case nil, .pending?:
                // A better ranked Mac has not answered yet: it may be the one.
                return QuickTodoSource(choice: choice, state: .loading)
            case .failed(let failure)?:
                lastFailure = failure
            case .projects(let reply)?:
                guard TodoMode(rawValue: reply.mode) == .icloud else { continue }
                var others: [String: TodoProjects] = [:]
                for mac in macs where mac.id != id {
                    if case .projects(let other)? = answers[mac.id], TodoMode(rawValue: other.mode) == .icloud { others[mac.id] = other }
                }
                return QuickTodoSource(
                    choice: choice, state: .projects(TodoRows.listed(reply, of: .icloud, servedBy: id, macs: macs, others: others)),
                    servedBy: macName(id)
                )
            }
        }
        // The Macs were there but did not answer, or none of them syncs.
        if let lastFailure { return QuickTodoSource(choice: choice, state: .failed(lastFailure)) }
        let anyOnline = macs.contains { $0.link == .online && $0.caps.todos }
        return QuickTodoSource(choice: choice, state: .failed(.noSyncMac(anyOnline: anyOnline)))
    }

    /// Opens the project list: takes what the Todos tab knows of the Macs by now, and asks every Mac that is
    /// online and knows todos for its projects, all at once. It returns at once and the list fills as they
    /// answer; the task ends when all of them have. The Macs that cannot be asked are told apart by the sources
    /// (offline, too old).
    @discardableResult
    public func startLoadingProjects() -> Task<Void, Never> {
        picking = true
        epoch += 1
        let mine = epoch
        storedModes = TodosStore.storedModes(defaults)
        lastICloudMac = defaults.string(forKey: TodosStore.icloudMacKey)
        let asked = Self.usable(macs)
        answers = Dictionary(uniqueKeysWithValues: asked.map { ($0, Answer.pending) })
        return Task { await ask(Array(asked), epoch: mine) }
    }

    /// `startLoadingProjects`, waited for.
    public func loadProjects() async {
        await startLoadingProjects().value
    }

    /// Asks a source again, after it did not answer or when the user pulls down: what the Macs already said
    /// stays on screen until the new answers are in.
    public func retry(_ source: TodoSource) async {
        let ids: [String]
        switch source {
        case .mac(let id): ids = Self.usable(macs).contains(id) ? [id] : []
        case .icloud: ids = Self.usable(macs).filter { modes[$0] != .local }.sorted()
        }
        guard !ids.isEmpty else { return }
        for id in ids {
            if case .failed? = answers[id] { answers[id] = .pending }
        }
        await ask(ids, epoch: epoch)
    }

    private func ask(_ ids: [String], epoch mine: Int) async {
        let tasks = ids.map { id in
            Task { @MainActor in
                let result = await self.fetch(id)
                self.adopt(result, from: id, epoch: mine)
            }
        }
        for task in tasks { await task.value }
    }

    /// The list is not on screen any more: answers still on their way are dropped.
    public func closeProjects() {
        picking = false
        epoch += 1
        query = ""
        emptyExpanded = false
    }

    private func fetch(_ macId: String) async -> Result<TodoProjects, Error> {
        do { return .success(try await backend.todoProjects(macId: macId)) } catch { return .failure(error) }
    }

    private func adopt(_ result: Result<TodoProjects, Error>, from macId: String, epoch mine: Int) {
        guard mine == epoch, macs.contains(where: { $0.id == macId }) else { return }
        switch result {
        case .success(let reply):
            learnedModes[macId] = TodoMode(rawValue: reply.mode) ?? .local
            answers[macId] = .projects(reply)
        case .failure(let error):
            answers[macId] = .failed(TodoFailure.of(error))
        }
    }
}
