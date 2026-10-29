import Foundation
import Observation
import AHProtocol

/// The project a screen has opened.
public struct OpenedTodoProject: Hashable, Sendable {
    /// The Mac that serves it (for the iCloud source: the one that answers for iCloud right now).
    public var macId: String
    public var id: String
    /// The row it was opened from (name, hint, repository, scope).
    public var row: TodoProjectRow

    var key: String { TodosStore.key(macId, id) }
}

/// A note the screen shows for a moment.
public struct TodoNoticeItem: Identifiable, Equatable, Sendable {
    public var id: Int
    public var notice: TodoNotice
}

/// The state behind the Todos tab (docs/specs/ios-todos.md §4, §7): which source is shown, the projects
/// and todos of it, and the writes the user makes. The Macs own the todos; this only shows what they
/// said, with the user's own pending changes laid over it until the Mac has answered.
///
/// Two rules keep it simple:
/// - one request per kind is in flight at a time (`kick`), a newer wish just reruns it;
/// - the list on screen is always *what the Mac last said* plus *the writes it has not answered yet*,
///   so a refused or lost write disappears by itself, and an answer that arrives while a write is in
///   flight cannot make the screen jump back.
@MainActor
@Observable
public final class TodosStore {
    // MARK: what the screens read

    public private(set) var macs: [MacState] = []
    public private(set) var source: TodoSource?
    /// The Mac answering for the source right now (`nil` while none does).
    public private(set) var accessMacId: String?
    public private(set) var projects: TodoProjects?
    public private(set) var projectsState: TodoLoadState = .idle
    /// What each Mac said it does with todos, learned from its answers and kept between launches (so the
    /// tab can start on iCloud without asking first).
    public private(set) var modes: [String: TodoMode] = [:]
    /// iCloud: what the other Macs behind it said last. The serving Mac's rows are `projects`; what only
    /// these Macs have (their projects without a repository) is listed with them.
    public private(set) var others: [String: TodoProjects] = [:]
    public var query = ""
    /// The "projects without todos" section is open.
    public var emptyExpanded = false
    public private(set) var opened: OpenedTodoProject?
    public private(set) var listStates: [String: TodoLoadState] = [:]
    public private(set) var notice: TodoNoticeItem?

    // MARK: private state

    /// What the Mac last said for each project (`macId|projectId`).
    private(set) var confirmed: [String: ProjectTodos] = [:]
    /// The user's writes the Mac has not answered, by project, oldest first.
    private(set) var overlay: [String: [PendingWrite]] = [:]

    struct PendingWrite: Equatable, Sendable {
        var id: String
        var mutation: TodoMutation
    }

    private struct QueuedWrite {
        var key: String
        var macId: String
        var projectId: String
        var write: PendingWrite
    }

    @ObservationIgnored private let backend: RemoteBackend
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let now: @Sendable () -> Date
    @ObservationIgnored private var visible = false
    @ObservationIgnored private var foreground = true
    @ObservationIgnored private var started = false
    @ObservationIgnored private var cache: [TodoSource: (macId: String, projects: TodoProjects)] = [:]
    @ObservationIgnored private var lastIcloudMac: String?
    /// The signature of each Mac's todo store as it last reported it: a `todo.changed` that carries it says
    /// nothing new (it is the echo of our own write, or of what was just loaded).
    @ObservationIgnored private var knownRevs: [String: UInt64] = [:]
    /// What the Agents are doing (`update(agents:)`): the dots of the project rows.
    private var activityIndex = TodoActivityIndex()
    /// The user pulled down: the next full load asks every other Mac what it does with todos, not just the unknown ones.
    @ObservationIgnored private var relearn = false
    @ObservationIgnored private var projectsDirty = false
    /// The pending load of the projects is a quiet look (see `armSyncRecheck`); a load anybody else asks for is not.
    @ObservationIgnored private var projectsQuiet = false
    @ObservationIgnored private var listDirty = false
    @ObservationIgnored private var runner: Task<Void, Never>?
    @ObservationIgnored private var queue: [QueuedWrite] = []
    @ObservationIgnored private var drainer: Task<Void, Never>?
    /// The Macs whose `todo.changed` the phone is subscribed to.
    @ObservationIgnored private var watching: Set<String> = []
    @ObservationIgnored private var noticeSerial = 0
    @ObservationIgnored private let recheckDelays: [Duration]
    @ObservationIgnored private var recheckTask: Task<Void, Never>?
    /// How many quiet looks the Mac's "syncing" has cost since it last said otherwise (or since the user asked again).
    @ObservationIgnored private(set) var rechecks = 0

    private static let sourceKey = "ah.todos.source"
    private static let icloudMacKey = "ah.todos.icloudMac"
    private static let modesKey = "ah.todos.modes"

    /// How long the phone waits before it asks again while the Mac says it is syncing: about 35 seconds in
    /// all. A pull that takes longer than that is left alone; pulling down asks again.
    public static let syncRecheckDelays: [Duration] = [
        .milliseconds(1500), .seconds(2), .seconds(3), .seconds(4), .seconds(5), .seconds(5), .seconds(5), .seconds(5), .seconds(5),
    ]

    public init(
        backend: RemoteBackend, defaults: UserDefaults = .standard, now: @escaping @Sendable () -> Date = { Date() },
        syncRecheckDelays: [Duration] = TodosStore.syncRecheckDelays
    ) {
        self.backend = backend
        self.defaults = defaults
        self.now = now
        self.recheckDelays = syncRecheckDelays
        lastIcloudMac = defaults.string(forKey: Self.icloudMacKey)
        modes = Self.storedModes(defaults)
    }

    nonisolated static func key(_ macId: String, _ projectId: String) -> String { macId + "|" + projectId }

    // MARK: derived

    public var choices: [TodoSourceChoice] { TodoSources.choices(macs: macs, modes: modes) }

    public var sections: [TodoSection] {
        guard let projects, let source else { return [] }
        return TodoRows.sections(listed: listed(projects, of: source), source: source, query: query, emptyExpanded: emptyExpanded)
    }

    /// The rows of the source, each with the Mac that told it. iCloud adds, from the other Macs behind it,
    /// the projects only they have: those without a repository, which no other Mac (and not iCloud) knows.
    private func listed(_ projects: TodoProjects, of source: TodoSource) -> [TodoListedProject] {
        func name(_ id: String) -> String { macs.first { $0.id == id }?.name ?? "" }
        let serving = accessMacId ?? ""
        var out = projects.projects.map { TodoListedProject(macId: serving, macName: name(serving), row: $0) }
        guard source == .icloud else { return out }
        for mac in macs where mac.id != serving {
            guard let reply = others[mac.id] else { continue }
            for var row in reply.projects where row.scope == "localOnly" || row.scope == "local" {
                // Whatever the Mac called its sections, here they are the ones of the iCloud view.
                row.section = (row.pending > 0 ? TodoSection.Kind.localOnly : .empty).rawValue
                out.append(TodoListedProject(macId: mac.id, macName: mac.name, row: row))
            }
        }
        return out
    }

    /// What the Agents are doing in a listed project, or `nil` when none is live in it.
    public func activity(of project: TodoListedProject) -> TodoActivity? { activityIndex.activity(of: project) }

    public var syncLine: TodoSyncLine? { projects?.sync.map(TodoSyncLine.derive) }

    /// The Mac that serves the source, when it is not obvious (iCloud: "via MacBook Pro").
    public var accessName: String? {
        guard case .icloud? = source, let id = accessMacId else { return nil }
        return macs.first { $0.id == id }?.name
    }

    /// The Mac that answers for the screen, whatever the source.
    public var accessMacName: String? {
        accessMacId.flatMap { id in macs.first { $0.id == id }?.name }
    }

    /// A paired Mac's name.
    public func macName(_ id: String?) -> String? {
        id.flatMap { id in macs.first { $0.id == id }?.name }
    }

    /// The Mac that serves the project that is open: with iCloud that can be another one than the Mac
    /// serving the list.
    public var openedMacName: String? { macName(opened?.macId) }

    public var sourceTitle: String? {
        switch source {
        case .mac(let id)?: return macs.first { $0.id == id }?.name
        default: return nil
        }
    }

    /// Whether the Mac behind the screen can take a write now. Writes are not queued for later.
    public var canWrite: Bool {
        guard let id = opened?.macId ?? accessMacId, let mac = macs.first(where: { $0.id == id }) else { return false }
        return mac.link == .online && mac.caps.todos
    }

    public var list: ProjectTodos? { opened.flatMap { list(for: $0.key) } }

    public var listState: TodoLoadState { opened.map { listStates[$0.key] ?? .idle } ?? .idle }

    /// How many writes of the open project wait for the Mac.
    public var writing: Int { opened.map { overlay[$0.key]?.count ?? 0 } ?? 0 }

    /// Whether the todo is one the user just added and the Mac has not confirmed.
    public func isSending(_ todoId: String) -> Bool {
        guard let key = opened?.key else { return false }
        return overlay[key]?.contains { $0.mutation.localId == todoId } ?? false
    }

    func list(for key: String) -> ProjectTodos? {
        guard var list = confirmed[key] else { return nil }
        let at = now()
        for write in overlay[key] ?? [] { write.mutation.apply(to: &list, now: at) }
        return list
    }

    // MARK: lifecycle

    /// The Todos tab came on screen or went away.
    public func setVisible(_ visible: Bool) {
        guard self.visible != visible else { return }
        self.visible = visible
        cancelSyncRecheck()
        if visible {
            ensureSource()
            kick(projects: true, list: opened != nil)
        }
        syncWatch()
    }

    /// Mirrors the app state: the paired Macs and whether the app is in the foreground.
    public func update(macs next: [MacState], foreground: Bool) {
        let before = macs
        let foregroundChanged = self.foreground != foreground
        self.foreground = foreground
        if foregroundChanged { cancelSyncRecheck() }
        if next != before {
            macs = next
            forgetUnpairedMacs()
        }
        guard started || visible else {
            syncWatch()
            return
        }
        let chosen = source
        ensureSource()
        func usable(_ list: [MacState]) -> Set<String> {
            Set(list.filter { $0.link == .online && $0.caps.todos }.map(\.id))
        }
        func down(_ list: [MacState]) -> Set<String> {
            Set(list.filter { $0.link == .macOffline || $0.link == .brokerDown }.map(\.id))
        }
        let came = usable(next).subtracting(usable(before))
        let went = usable(before).subtracting(usable(next))
        if visible {
            // A source picked only now (the tab was on screen before the Macs were known) has nothing loaded
            // yet, even if its Mac cannot answer: an outdated or unreachable Mac is explained, not left loading.
            var reload = source != chosen
            switch source {
            case .mac(let id)?:
                reload = reload || came.contains(id)
                if down(next).contains(id) { projectsState = .failed(.offline) }
            case .icloud?:
                // Something that can serve iCloud came up, or the one serving it left.
                reload = reload || !came.isEmpty || (accessMacId.map { went.contains($0) } ?? false)
            case nil:
                break
            }
            if reload { kick(projects: true, list: opened != nil) }
        }
        if foregroundChanged, foreground, visible { kick(projects: true, list: opened != nil) }
        syncWatch()
    }

    /// The Agents of the paired Macs, as the Agents tab lists them: the dots of the project rows follow them.
    public func update(agents: [AgentItem]) {
        let next = TodoActivityIndex(agents: agents)
        if next != activityIndex { activityIndex = next }
    }

    /// What was kept of Macs that are not paired any more goes.
    private func forgetUnpairedMacs() {
        guard !macs.isEmpty else { return }
        let paired = Set(macs.map(\.id))
        if others.keys.contains(where: { !paired.contains($0) }) { others = others.filter { paired.contains($0.key) } }
        if modes.keys.contains(where: { !paired.contains($0) }) {
            modes = modes.filter { paired.contains($0.key) }
            storeModes()
        }
    }

    /// `todo.changed` of a Mac. Only the Mac serving the list and the one serving the open project are watched.
    public func changed(macId: String, rev: UInt64) {
        guard visible, macId == accessMacId || macId == opened?.macId, rev != knownRevs[macId] else { return }
        kick(projects: true, list: opened != nil)
    }

    // MARK: source

    public func select(_ choice: TodoSource) {
        var new = choice
        // A Mac behind iCloud has no view of its own.
        if case .mac(let id) = new, TodoSources.isAbsorbed(id, macs: macs, modes: modes) { new = .icloud }
        guard new != source else { return }
        defaults.set(new.stored, forKey: Self.sourceKey)
        setSource(new)
        kick(projects: true)
    }

    private func setSource(_ new: TodoSource?) {
        source = new
        started = true
        cancelSyncRecheck()
        opened = nil
        query = ""
        emptyExpanded = false
        knownRevs = [:]
        if let new, let hit = cache[new] {
            projects = hit.projects
            accessMacId = hit.macId
            projectsState = .loaded
        } else {
            projects = nil
            accessMacId = nil
            projectsState = new == nil ? .failed(.noMacs) : .loading
        }
        syncWatch()
    }

    /// Picks the source when none is chosen yet or the chosen Mac is gone.
    private func ensureSource() {
        started = true
        if macs.isEmpty {
            if source != nil { setSource(nil) }
            projectsState = .failed(.noMacs)
            return
        }
        if case .mac(let id)? = source, macs.contains(where: { $0.id == id }),
            !TodoSources.isAbsorbed(id, macs: macs, modes: modes) {
            return
        }
        if case .icloud? = source { return }
        let stored = defaults.string(forKey: Self.sourceKey).flatMap(TodoSource.init(stored:))
        setSource(TodoSources.initial(stored: stored, macs: macs, modes: modes))
    }

    // MARK: what the Macs do

    /// A Mac said what it does with todos. A Mac that stops syncing leaves iCloud's rows behind.
    private func learn(_ mode: TodoMode, of macId: String) {
        guard modes[macId] != mode else { return }
        modes[macId] = mode
        storeModes()
        if mode == .local { others[macId] = nil }
    }

    private func storeModes() {
        let raw = modes.mapValues(\.rawValue)
        if let data = try? JSONEncoder().encode(raw) { defaults.set(data, forKey: Self.modesKey) }
    }

    private static func storedModes(_ defaults: UserDefaults) -> [String: TodoMode] {
        guard let data = defaults.data(forKey: modesKey),
            let raw = try? JSONDecoder().decode([String: String].self, from: data)
        else { return [:] }
        return raw.compactMapValues(TodoMode.init(rawValue:))
    }

    // MARK: loading

    /// Pull to refresh: loads again and returns when the answers are in.
    public func refresh() async {
        // The user asked again: the Mac's "syncing" gets its looks back, and the other Macs say again what they
        // do with todos (one may have started or stopped syncing since the phone last asked).
        rechecks = 0
        relearn = true
        kick(projects: true, list: opened != nil)
        await runner?.value
    }

    private func kick(projects: Bool = false, list: Bool = false, quiet: Bool = false) {
        if projects {
            projectsQuiet = projectsDirty ? (projectsQuiet && quiet) : quiet
            projectsDirty = true
        }
        if list { listDirty = true }
        guard runner == nil, visible || started else { return }
        runner = Task { [weak self] in await self?.run() }
    }

    private func run() async {
        while projectsDirty || listDirty {
            var asked = false
            if projectsDirty {
                projectsDirty = false
                let quiet = projectsQuiet
                projectsQuiet = false
                await loadProjects(quiet: quiet)
                asked = !quiet
            }
            if listDirty {
                listDirty = false
                await loadList()
            }
            if !projectsDirty, !listDirty { await loadOthers(refresh: asked) }
        }
        runner = nil
    }

    /// A quiet load is a look the user did not ask for: when it gets no answer, nothing on screen changes.
    private func loadProjects(quiet: Bool = false) async {
        guard let source else { return }
        if projects == nil { projectsState = .loading }
        switch source {
        case .mac(let id):
            guard let mac = macs.first(where: { $0.id == id }) else { return }
            guard mac.caps.todos else { return fail(.needsUpdate, for: source) }
            guard mac.link == .online else { return fail(.offline, for: source) }
            do {
                let reply = try await backend.todoProjects(macId: id)
                guard self.source == source else { return }
                adopt(reply, from: id, for: source)
            } catch {
                guard self.source == source else { return }
                if quiet { armSyncRecheck() } else { fail(TodoFailure.of(error), for: source) }
            }
        case .icloud:
            if quiet, let id = accessMacId {
                // Only the Mac that answered is asked, once; the usual search for another Mac is for loads the user wants.
                let answer = try? await backend.todoProjects(macId: id)
                guard self.source == source, accessMacId == id else { return }
                // No answer changes nothing on screen; the next look is already due.
                guard let reply = answer else { return armSyncRecheck() }
                let mode = TodoMode(rawValue: reply.mode) ?? .local
                learn(mode, of: id)
                if mode == .icloud { return adopt(reply, from: id, for: source) }
                // It stopped syncing: iCloud may have to move to another Mac, which the usual load works out.
                return kick(projects: true)
            }
            var failed: Set<String> = []
            // Why the Macs that were asked did not serve it, when the asking is what failed (a Mac that is not
            // there to ask is a different matter).
            var lastFailure: TodoFailure?
            while true {
                let candidates = TodoSources.icloudCandidates(macs: macs, modes: modes, lastUsed: lastIcloudMac, failed: failed)
                guard let id = candidates.first else {
                    // The Mac whose rows are on screen is still paired but out of reach, and no other Mac can take over:
                    // its rows (and the project that is open) stay as it last said them, read-only and with the page's note,
                    // the way they do when that Mac is the source itself. An unpaired Mac leaves nothing behind.
                    if projects != nil, let serving = accessMacId,
                        let mac = macs.first(where: { $0.id == serving }), mac.link != .online {
                        return fail(.offline, for: source)
                    }
                    // The Macs were there but did not answer: the rows on screen stay as what one of them last said, with
                    // the page's note, and a slow Mac is not called "not syncing". With no rows to keep, the page says why.
                    if let lastFailure { return fail(lastFailure, for: source) }
                    // Nothing serves iCloud now: rows of the Mac that did cannot be opened any more.
                    accessMacId = nil
                    projects = nil
                    opened = nil
                    let anyOnline = macs.contains { $0.link == .online && $0.caps.todos }
                    return fail(.noSyncMac(anyOnline: anyOnline), for: source)
                }
                do {
                    let reply = try await backend.todoProjects(macId: id)
                    guard self.source == source else { return }
                    let mode = TodoMode(rawValue: reply.mode) ?? .local
                    learn(mode, of: id)
                    if mode == .icloud {
                        lastIcloudMac = id
                        defaults.set(id, forKey: Self.icloudMacKey)
                        return adopt(reply, from: id, for: source)
                    }
                } catch {
                    guard self.source == source else { return }
                    failed.insert(id)
                    lastFailure = TodoFailure.of(error)
                }
            }
        }
    }

    private func fail(_ failure: TodoFailure, for source: TodoSource) {
        guard self.source == source else { return }
        projectsState = .failed(failure)
        // A source that cannot be reached must not keep another Mac's watch open.
        if projects == nil { accessMacId = nil }
        syncWatch()
    }

    private func adopt(_ reply: TodoProjects, from macId: String, for source: TodoSource) {
        learn(TodoMode(rawValue: reply.mode) ?? .local, of: macId)
        // A Mac that was shown on its own turns out to sync, with others paired: its view is iCloud's now.
        // The answer in hand is iCloud's first one, so nothing flickers.
        if case .mac(let id) = source, id == macId, TodoSources.isAbsorbed(macId, macs: macs, modes: modes) {
            lastIcloudMac = macId
            defaults.set(macId, forKey: Self.icloudMacKey)
            setSource(.icloud)
            return adopt(reply, from: macId, for: .icloud)
        }
        // iCloud moved to another Mac: what the one that served it said stays, as one of the others behind it.
        if source == .icloud, let previous = accessMacId, previous != macId, let old = projects,
            modes[previous] == .icloud, macs.contains(where: { $0.id == previous }) {
            others[previous] = old
        }
        // A project served by another Mac than the one answering now (iCloud moved over) cannot stay, unless it
        // exists on that Mac only: that Mac goes on serving it.
        if let current = opened, current.macId != macId, current.row.scope != "localOnly", current.row.scope != "local" {
            opened = nil
        }
        others[macId] = nil
        projects = reply
        projectsState = .loaded
        accessMacId = macId
        knownRevs[macId] = reply.rev
        cache[source] = (macId, reply)
        syncWatch()
        armSyncRecheck()
    }

    // MARK: the sync line

    /// The Mac's `syncing` is a snapshot of the instant it answered. Opening the tab starts a pull on the Mac
    /// (the subscription refreshes a stale replica), so the first answer usually catches it, and nothing tells
    /// the phone when the pull ends: the todo file often does not change, so no `todo.changed` follows. The
    /// phone therefore looks again, quietly and a few times, until the line stops saying "Syncing…".
    private func armSyncRecheck() {
        recheckTask?.cancel()
        recheckTask = nil
        guard visible, foreground, source != nil, projects?.sync?.syncing == true else {
            rechecks = 0
            return
        }
        guard rechecks < recheckDelays.count else { return }
        let delay = recheckDelays[rechecks]
        rechecks += 1
        recheckTask = Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled, let self else { return }
            self.recheckTask = nil
            guard self.visible, self.foreground else { return }
            self.kick(projects: true, quiet: true)
        }
    }

    /// The tab went away, the app left the foreground or the source changed: the next load starts afresh.
    private func cancelSyncRecheck() {
        recheckTask?.cancel()
        recheckTask = nil
        rechecks = 0
    }

    private func loadList() async {
        guard let target = opened else { return }
        let key = target.key
        // An answer that predates a pending write would make the list jump back, so none is asked for or
        // taken while writes wait; the end of the writes asks again (`drain`).
        guard (overlay[key] ?? []).isEmpty else { return }
        if confirmed[key] == nil { listStates[key] = .loading }
        do {
            let reply = try await backend.todoList(macId: target.macId, projectId: target.id)
            guard opened == target else { return }
            guard reply.found else {
                confirmed[key] = nil
                listStates[key] = .failed(.other("gone"))
                return
            }
            guard (overlay[key] ?? []).isEmpty else { return }
            confirmed[key] = reply
            listStates[key] = .loaded
            knownRevs[target.macId] = reply.rev
        } catch {
            guard opened == target else { return }
            listStates[key] = .failed(TodoFailure.of(error))
        }
    }

    /// Asks the Macs that are not the one serving the screen, one at a time and only while nothing else wants
    /// the connection: a Mac whose mode is not known yet is learned (the source menu needs it), and with
    /// `refresh` the other Macs behind iCloud say again which projects only they have. After the user pulled
    /// down (`relearn`) every Mac is asked again, whatever the phone believed it did.
    private func loadOthers(refresh: Bool) async {
        for mac in macs where mac.link == .online && mac.caps.todos && mac.id != accessMacId {
            let known = modes[mac.id]
            let again = refresh && (relearn || (source == .icloud && known == .icloud))
            guard known == nil || again else { continue }
            guard !projectsDirty, !listDirty, visible else { return }
            guard let reply = try? await backend.todoProjects(macId: mac.id) else { continue }
            guard macs.contains(where: { $0.id == mac.id }), mac.id != accessMacId else { continue }
            let mode = TodoMode(rawValue: reply.mode) ?? .local
            learn(mode, of: mac.id)
            if mode == .icloud {
                others[mac.id] = reply
                knownRevs[mac.id] = reply.rev
            } else {
                cache[.mac(mac.id)] = (mac.id, reply)
            }
        }
        if refresh { relearn = false }
    }

    // MARK: opening projects

    /// Opens a project on the Mac that listed it: with iCloud that is not always the Mac serving the list.
    public func open(_ project: TodoListedProject) {
        guard !project.macId.isEmpty else { return }
        let target = OpenedTodoProject(macId: project.macId, id: project.id, row: project.row)
        opened = target
        listStates[target.key] = confirmed[target.key] == nil ? .loading : .loaded
        kick(list: true)
        syncWatch()
    }

    public func close() {
        opened = nil
        syncWatch()
    }

    public func reloadList() {
        guard opened != nil else { return }
        kick(list: true)
    }

    // MARK: writes

    /// Adds a todo at the end of the project's queue. `nil` when it was taken; else why not.
    @discardableResult
    public func add(_ raw: String, auto: Bool) -> TodoTextError? {
        switch TodoText.validate(raw) {
        case .failure(let error): return error
        case .success(let text):
            write(.add(localId: "local-" + Bytes.newMsgId(), text: text, auto: auto))
            return nil
        }
    }

    @discardableResult
    public func edit(_ id: String, text raw: String) -> TodoTextError? {
        switch TodoText.validate(raw) {
        case .failure(let error): return error
        case .success(let text):
            write(.edit(id: id, text: text))
            return nil
        }
    }

    public func complete(_ id: String) { write(.complete(id: id)) }
    public func restore(_ id: String) { write(.restore(id: id)) }
    public func remove(_ id: String) { write(.remove(id: id)) }
    public func setAuto(_ id: String, _ auto: Bool) { write(.setAuto(id: id, auto: auto)) }

    /// The pending todos in their new order (SwiftUI's `onMove`).
    public func move(from source: IndexSet, to destination: Int) {
        guard let current = list?.pending else { return }
        let order = Self.moved(current.map(\.id), from: source, to: destination)
        write(.reorder(ids: order.filter { !$0.hasPrefix("local-") }))
    }

    /// `onMove` semantics: the elements at `source` end up before the element that was at `destination`.
    nonisolated static func moved(_ ids: [String], from source: IndexSet, to destination: Int) -> [String] {
        var out = ids
        let moving = source.sorted().filter { $0 < ids.count }.map { ids[$0] }
        let before = source.filter { $0 < destination }.count
        for index in source.sorted(by: >) where index < out.count { out.remove(at: index) }
        out.insert(contentsOf: moving, at: min(max(destination - before, 0), out.count))
        return out
    }

    private func write(_ mutation: TodoMutation) {
        guard let target = opened else { return }
        guard canWrite, confirmed[target.key] != nil else {
            post(.offline)
            return
        }
        let pending = PendingWrite(id: Bytes.newMsgId(), mutation: mutation)
        overlay[target.key, default: []].append(pending)
        queue.append(QueuedWrite(key: target.key, macId: target.macId, projectId: target.id, write: pending))
        guard drainer == nil else { return }
        drainer = Task { [weak self] in await self?.drain() }
    }

    private func drain() async {
        while !queue.isEmpty {
            let item = queue.removeFirst()
            await send(item)
        }
        drainer = nil
        // What the writes did to the counts and sections, and whatever changed meanwhile: ask again, quietly.
        kick(projects: true, list: opened != nil)
    }

    private func send(_ item: QueuedWrite) async {
        let operation = item.write.mutation.operation(clientMsgId: item.write.id, projectId: item.projectId)
        var ack: TodoAck?
        var failure: Error?
        // A lost answer is asked for again once; the id makes the repeat harmless.
        for _ in 0..<2 {
            do {
                ack = try await backend.todoApply(macId: item.macId, operation: operation)
                break
            } catch BackendError.timedOut {
                failure = BackendError.timedOut
            } catch {
                failure = error
                break
            }
        }
        settle(item, ack: ack)
        guard let ack else {
            let offline = (failure as? BackendError).map { $0 == .macOffline || $0 == .unknownMac } ?? false
            post(offline ? .offline : .unconfirmed)
            if offline { dropQueued(for: item.macId) }
            reloadSoon(item.key)
            return
        }
        switch ack.status {
        case "ok": break
        case "notFound": post(.changedElsewhere)
        case "rejected": post(.refused(ack.reason ?? ""))
        default: post(.notSaved)
        }
        if ack.list == nil { reloadSoon(item.key) }
    }

    /// The Mac answered (or not): the write leaves the overlay and the Mac's list takes its place.
    private func settle(_ item: QueuedWrite, ack: TodoAck?) {
        overlay[item.key]?.removeAll { $0.id == item.write.id }
        if overlay[item.key]?.isEmpty == true { overlay[item.key] = nil }
        guard let list = ack?.list, list.found else { return }
        confirmed[item.key] = list
        listStates[item.key] = .loaded
        knownRevs[item.macId] = list.rev
        // The row counts follow at once; the sections follow with the next load of the projects.
        func counted(_ reply: TodoProjects?) -> TodoProjects? {
            guard var current = reply, let index = current.projects.firstIndex(where: { $0.id == item.projectId }) else { return nil }
            current.projects[index].pending = list.pendingTotal
            current.projects[index].history = list.historyTotal
            return current
        }
        if item.macId == accessMacId {
            if let current = counted(projects) { projects = current }
        } else if let current = counted(others[item.macId]) {
            others[item.macId] = current
        }
    }

    /// The Mac cannot be reached: nothing queued for it will go out.
    private func dropQueued(for macId: String) {
        for item in queue where item.macId == macId {
            overlay[item.key]?.removeAll { $0.id == item.write.id }
            if overlay[item.key]?.isEmpty == true { overlay[item.key] = nil }
        }
        queue.removeAll { $0.macId == macId }
    }

    private func reloadSoon(_ key: String) {
        if opened?.key == key { kick(list: true) }
    }

    // MARK: notices

    private func post(_ notice: TodoNotice) {
        noticeSerial += 1
        self.notice = TodoNoticeItem(id: noticeSerial, notice: notice)
    }

    public func dismissNotice(_ id: Int) {
        if notice?.id == id { notice = nil }
    }

    // MARK: subscription

    /// Keeps the Macs' `todo.changed` subscriptions in step with what is on screen: only the Mac serving the
    /// list and the one serving the open project (with iCloud it can be another one) of a visible Todos tab
    /// in a foreground app are asked.
    private func syncWatch() {
        var want: Set<String> = []
        if visible, foreground {
            if let accessMacId { want.insert(accessMacId) }
            if let opened { want.insert(opened.macId) }
        }
        guard want != watching else { return }
        let dropped = watching.subtracting(want)
        let added = want.subtracting(watching)
        watching = want
        let backend = backend
        Task {
            for id in dropped.sorted() { await backend.todoWatch(macId: id, on: false) }
            for id in added.sorted() { await backend.todoWatch(macId: id, on: true) }
        }
    }
}
