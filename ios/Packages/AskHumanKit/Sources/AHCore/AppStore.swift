import Foundation
import Observation
import AHProtocol
import AHStore

public enum AppTab: String, Hashable, Sendable, CaseIterable {
    case inbox, agents, history, macs
}

public enum InboxFilter: Hashable, Sendable {
    case all
    case mac(String)
    case asks
    case confirms
}

/// A group of pending requests of one project on one Mac.
public struct InboxSection: Identifiable, Equatable, Sendable {
    public var id: String
    public var projectName: String
    public var macId: String
    public var macName: String
    public var items: [InboxItem]
    public var latest: Date
}

/// What became of a request that disappeared from the inbox while the user had it open.
public struct FinishedRequest: Equatable, Sendable {
    public var outcome: HistoryOutcome
    public var by: String?
    public var at: Date
}

/// A short message over the app: a new request, a failure, a notice.
public struct Banner: Identifiable, Equatable, Sendable {
    public enum Kind: Sendable, Equatable {
        case request(macId: String, requestId: String)
        /// A followed Agent finished (`done`), failed (`failed`) or asks something (`waiting`).
        case agent(macId: String, agentKey: String, event: String)
        /// The Mac already follows as many Agents as it allows.
        case followLimit
        case info
        case warning
    }

    public var id = UUID()
    public var kind: Kind
    public var title: String
    public var subtitle: String?

    public init(kind: Kind, title: String, subtitle: String? = nil) {
        self.kind = kind
        self.title = title
        self.subtitle = subtitle
    }
}

/// Everything the screens observe. It follows a `RemoteBackend` (the real engine or the mock),
/// derives the lists the UI needs and carries the user's actions back.
@MainActor
@Observable
public final class AppStore {
    public private(set) var state = EngineState()
    public private(set) var history: [HistoryEntry] = []
    /// Requests that ended while open, by inbox item id.
    public private(set) var finished: [String: FinishedRequest] = [:]
    /// Bumps for every request that arrives while the app is in the foreground (the UI plays a haptic).
    public internal(set) var arrivalTick = 0
    public private(set) var started = false

    public var tab: AppTab = .inbox
    public var filter: InboxFilter = .all
    /// Ids (`macId/requestId`) of the requests pushed on the inbox stack. A request that leaves the
    /// stack is closed: its draft is saved, or dropped when the request is over.
    public var inboxPath: [String] = [] {
        didSet {
            for id in Set(oldValue).subtracting(inboxPath) { closed(id: id) }
        }
    }
    public var banner: Banner?
    /// Ids (`macId|agentKey`) of the Agents pushed on the Agents stack. The live frames of an Agent
    /// are asked for while its page is on the stack.
    public var agentPath: [String] = [] {
        didSet {
            for id in Set(agentPath).subtracting(oldValue) { setWatch(id, open: true) }
            for id in Set(oldValue).subtracting(agentPath) { setWatch(id, open: false) }
        }
    }
    public var agentFilter: AgentFilter = .all
    /// When the Agent of an Agent page last picked up the messages sent to it (by `AgentItem.id`).
    public private(set) var consumed: [String: Date] = [:]
    /// The new task form while its sheet is open.
    public var taskComposer: TaskComposer?
    /// The task whose progress the sheet shows (after sending it, or when opened from the list).
    public var viewingTask: String?
    /// Jump to the next pending request after a submission.
    public var autoAdvance: Bool {
        didSet { defaults.set(autoAdvance, forKey: Self.autoAdvanceKey) }
    }

    @ObservationIgnored public let backend: RemoteBackend
    @ObservationIgnored public let drafts: DraftStore
    @ObservationIgnored let defaults: UserDefaults
    @ObservationIgnored private var composers: [String: AnswerComposer] = [:]
    @ObservationIgnored private var confirmComposers: [String: ConfirmComposer] = [:]
    @ObservationIgnored private var tasks: [Task<Void, Never>] = []
    @ObservationIgnored private var lastHistoryRevision = -1
    /// Every request seen in the inbox, kept while its page may still be open: when the request ends
    /// the page keeps showing it (with a notice) instead of going blank.
    @ObservationIgnored private var retained: [String: (item: InboxItem, at: Date)] = [:]
    @ObservationIgnored private var bannerTask: Task<Void, Never>?
    /// Arrivals whose request is not in the published state yet (events and states travel apart).
    @ObservationIgnored private var pendingArrivals: [String: (macId: String, requestId: String)] = [:]
    /// Every Agent seen on the Agents tab, kept while its page may still be open.
    @ObservationIgnored var retainedAgents: [String: (item: AgentItem, at: Date)] = [:]
    @ObservationIgnored var interjectComposers: [String: InterjectComposer] = [:]

    private static let autoAdvanceKey = "ah.autoAdvance"

    public init(backend: RemoteBackend, drafts: DraftStore, defaults: UserDefaults = .standard) {
        self.backend = backend
        self.drafts = drafts
        self.defaults = defaults
        autoAdvance = defaults.object(forKey: Self.autoAdvanceKey) as? Bool ?? true
    }

    // MARK: lifecycle

    public func start() async {
        guard !started else { return }
        started = true
        // Subscribe before starting so that the first state is not missed.
        let states = backend.states
        let events = backend.events
        tasks.append(Task { [weak self] in
            for await next in states { await self?.apply(next) }
        })
        tasks.append(Task { [weak self] in
            for await event in events { await self?.handle(event) }
        })
        await backend.start()
    }

    public func setForeground(_ foreground: Bool) async {
        await backend.setForeground(foreground)
    }

    public func refresh() async {
        await backend.refresh()
    }

    private func apply(_ next: EngineState) async {
        state = next
        let now = Date()
        for item in next.inbox { retained[item.id] = (item, now) }
        for (key, entry) in retained where now.timeIntervalSince(entry.at) > 600 && !inboxPath.contains(key) {
            retained[key] = nil
        }
        if next.historyRevision != lastHistoryRevision {
            lastHistoryRevision = next.historyRevision
            history = await backend.history()
        }
        // Drop composers of requests that left the inbox and are not open any more.
        let live = Set(next.inbox.map(\.id))
        for key in composers.keys where !live.contains(key) && !inboxPath.contains(key) {
            composers[key] = nil
        }
        for key in confirmComposers.keys where !live.contains(key) && !inboxPath.contains(key) {
            confirmComposers[key] = nil
        }
        taskOptionsArrived()
        for item in next.agents { retainedAgents[item.id] = (item, now) }
        for (key, entry) in retainedAgents where now.timeIntervalSince(entry.at) > 600 && !agentPath.contains(key) {
            retainedAgents[key] = nil
        }
        for key in interjectComposers.keys where retainedAgents[key] == nil && !agentPath.contains(key) {
            interjectComposers[key] = nil
        }
        presentArrivals()
    }

    /// Shows a banner for the newest arrival whose request is known by now.
    private func presentArrivals() {
        var newest: InboxItem?
        for (key, _) in pendingArrivals {
            guard let item = state.inbox.first(where: { $0.id == key }) else { continue }
            pendingArrivals[key] = nil
            if newest == nil || item.receivedAt > newest!.receivedAt { newest = item }
        }
        guard let item = newest else { return }
        show(Banner(
            kind: .request(macId: item.macId, requestId: item.requestId),
            title: item.summary.agent.label + " · " + item.summary.project.name,
            subtitle: item.summary.preview
        ))
    }

    private func handle(_ event: EngineEvent) async {
        switch event {
        case .requestArrived(let macId, let requestId):
            let key = CachedRequest.key(macId, requestId)
            guard state.foreground, inboxPath.last != key else { return }
            arrivalTick += 1
            pendingArrivals[key] = (macId, requestId)
            presentArrivals()
        case .requestFinished(let macId, let requestId, let outcome, let by):
            let key = CachedRequest.key(macId, requestId)
            pendingArrivals[key] = nil
            if inboxPath.contains(key) {
                finished[key] = FinishedRequest(outcome: outcome, by: by, at: Date())
                // Our own submission went through: move on.
                if by == nil, outcome == .answered || outcome == .cancelled || outcome == .dismissed,
                   let sent = retained[key]?.item, case .sending = sent.state {
                    advance(from: key)
                }
            }
            if case .request(let bannerMac, let bannerId)? = banner?.kind, bannerMac == macId, bannerId == requestId {
                banner = nil
            }
        case .macUnpaired(let macId, let macName):
            pendingArrivals = pendingArrivals.filter { $0.value.macId != macId }
            show(Banner(kind: .warning, title: macName))
        case .pairingFinished:
            break
        case .followEvent(let macId, let event):
            presentFollowEvent(macId: macId, event: event)
        case .followRefused:
            show(Banner(kind: .followLimit, title: ""))
        case .interjectConsumed(let macId, let agentKey):
            consumed[AgentItem.key(macId, agentKey)] = Date()
        }
    }

    public func show(_ banner: Banner, for seconds: Double = 5) {
        self.banner = banner
        bannerTask?.cancel()
        let id = banner.id
        bannerTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled else { return }
            if self?.banner?.id == id { self?.banner = nil }
        }
    }

    public func dismissBanner() {
        banner = nil
    }

    // MARK: inbox

    public var macs: [MacState] { state.macs }
    public var hasPairedMacs: Bool { !state.macs.isEmpty }

    /// Pending requests that satisfy the filter, newest first.
    public func inboxItems(filter: InboxFilter? = nil) -> [InboxItem] {
        let active = filter ?? self.filter
        return state.inbox.filter { item in
            switch active {
            case .all: return true
            case .mac(let id): return item.macId == id
            case .asks: return item.summary.kind == "ask"
            case .confirms: return item.summary.kind == "confirm"
            }
        }
    }

    /// Requests grouped by project, groups and items newest first (spec §6.2).
    public func inboxSections(filter: InboxFilter? = nil) -> [InboxSection] {
        var order: [String] = []
        var groups: [String: InboxSection] = [:]
        for item in inboxItems(filter: filter) {
            let id = item.macId + "/" + item.summary.project.key
            if groups[id] == nil {
                order.append(id)
                groups[id] = InboxSection(
                    id: id, projectName: item.summary.project.name, macId: item.macId, macName: item.macName,
                    items: [], latest: item.receivedAt
                )
            }
            groups[id]?.items.append(item)
            if let latest = groups[id]?.latest, item.receivedAt > latest { groups[id]?.latest = item.receivedAt }
        }
        return order.compactMap { groups[$0] }
            .map { section in
                var section = section
                section.items.sort { $0.receivedAt > $1.receivedAt }
                return section
            }
            .sorted { $0.latest > $1.latest }
    }

    public func item(id: String) -> InboxItem? {
        state.inbox.first { $0.id == id }
    }

    /// The request for a page: the live one, or the last known version of one that just ended.
    public func displayItem(id: String) -> InboxItem? {
        item(id: id) ?? retained[id]?.item
    }

    public func isOnline(macId: String) -> Bool {
        state.macs.first { $0.id == macId }?.link == .online
    }

    /// After a submission went through: show the next pending request, or return to the list.
    public func advance(from id: String) {
        guard inboxPath.last == id else { return }
        if autoAdvance, let next = nextPending(after: id) {
            inboxPath = [next.id]
        } else {
            inboxPath = []
        }
    }

    public var pendingCount: Int { state.inbox.count }
    public var unreadCount: Int { state.inbox.filter(\.unread).count }

    public func hasDraft(for item: InboxItem) -> Bool {
        if let composer = composers[item.id] { return composer.hasContent }
        return drafts.draft(key: item.id)?.isEmpty == false
    }

    /// The next request to look at after `id` was dealt with: the oldest unread, else the newest.
    public func nextPending(after id: String?) -> InboxItem? {
        let candidates = state.inbox.filter { $0.id != id && $0.state == .pending }
        return candidates.first(where: \.unread) ?? candidates.first
    }

    // MARK: opening requests

    /// Pushes a request on the inbox stack (from a tap, a banner or a notification).
    public func open(macId: String, requestId: String) {
        tab = .inbox
        let key = CachedRequest.key(macId, requestId)
        guard inboxPath.last != key else { return }
        inboxPath = [key]
    }

    public func opened(_ item: InboxItem) async {
        if item.unread { await backend.markSeen(macId: item.macId, requestId: item.requestId) }
    }

    public func composer(for item: InboxItem) -> AnswerComposer? {
        guard let ask = item.view?.ask else { return nil }
        if let existing = composers[item.id] { return existing }
        let composer = AnswerComposer(key: item.id, ask: ask, drafts: drafts)
        composers[item.id] = composer
        return composer
    }

    public func confirmComposer(for item: InboxItem) -> ConfirmComposer? {
        guard let confirm = item.view?.confirm else { return nil }
        if let existing = confirmComposers[item.id] { return existing }
        let composer = ConfirmComposer(confirm: confirm)
        confirmComposers[item.id] = composer
        return composer
    }

    /// The form left the screen: save, and forget the in-memory state of finished requests.
    public func closed(id: String) {
        composers[id]?.flush()
        if item(id: id) == nil {
            composers[id]?.discard()
            composers[id] = nil
            confirmComposers[id] = nil
            finished[id] = nil
            retained[id] = nil
        }
    }

    /// Downloads an attachment of a request.
    public func download(macId: String, asset: AssetRef, progress: @escaping @Sendable (Double) -> Void) async throws -> URL {
        try await backend.download(macId: macId, asset: asset, progress: progress)
    }

    // MARK: actions

    /// Uploads the attachments and sends the answers.
    public func send(_ composer: AnswerComposer, item: InboxItem) async throws {
        composer.isSending = true
        composer.uploadProgress = nil
        defer {
            composer.isSending = false
            composer.uploadProgress = nil
        }
        var uploads: [String: String] = [:]
        let attachments = composer.answers.flatMap(\.attachments)
        for (index, attachment) in attachments.enumerated() {
            let data = try Data(contentsOf: composer.url(of: attachment))
            let source = UploadSource(name: attachment.name, mime: attachment.mime, data: data, isImage: attachment.isImage)
            let total = Double(attachments.count)
            let base = Double(index)
            uploads[attachment.id] = try await backend.upload(
                macId: item.macId, requestId: item.requestId, source: source,
                progress: { fraction in
                    Task { @MainActor in composer.uploadProgress = (base + fraction) / total }
                }
            )
        }
        _ = try await backend.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: composer.items(uploads: uploads)
        )
    }

    public func cancel(_ item: InboxItem) async throws {
        _ = try await backend.submitAnswer(macId: item.macId, requestId: item.requestId, action: "cancel", answers: [])
    }

    public func decide(_ composer: ConfirmComposer, item: InboxItem) async throws {
        guard let wireIndex = composer.selectedWireIndex else { return }
        composer.isSending = true
        defer { composer.isSending = false }
        _ = try await backend.submitConfirm(
            macId: item.macId, requestId: item.requestId, wireIndex: wireIndex, dismiss: false, input: composer.inputToSend
        )
    }

    public func dismiss(_ composer: ConfirmComposer, item: InboxItem) async throws {
        composer.isSending = true
        defer { composer.isSending = false }
        _ = try await backend.submitConfirm(macId: item.macId, requestId: item.requestId, wireIndex: nil, dismiss: true, input: nil)
    }

    public func retry(_ item: InboxItem) async throws {
        if case .failed(let clientMsgId, _) = item.state { try await backend.retrySubmission(clientMsgId: clientMsgId) }
    }

    public func discardFailure(_ item: InboxItem) async {
        if case .failed(let clientMsgId, _) = item.state { await backend.discardSubmission(clientMsgId: clientMsgId) }
    }

    // MARK: history

    public func deleteHistory(id: String) async { await backend.deleteHistory(id: id) }
    public func clearHistory() async { await backend.clearHistory() }

    // MARK: pairing and Macs

    public func startPairing(code: String) async { await backend.startPairing(code: code) }
    public func cancelPairing() async { await backend.cancelPairing() }
    public func finishPairing() async { await backend.finishPairing() }
    public func unpair(_ mac: MacState) async { await backend.unpair(macId: mac.id) }
    public func rename(_ mac: MacState, to alias: String?) async { await backend.renameMac(macId: mac.id, alias: alias) }
    public func forgetAll() async {
        inboxPath = []
        agentPath = []
        taskComposer = nil
        await backend.forgetAll()
    }

    public func updatePushToken(_ token: Data?) async { await backend.updatePushToken(token) }
    public func setDeviceName(_ name: String) async { await backend.setDeviceName(name) }
}
