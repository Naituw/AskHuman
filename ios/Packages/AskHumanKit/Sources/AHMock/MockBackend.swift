import Foundation
import AHProtocol
import AHCore
import AHStore

/// An in-memory `RemoteBackend` with sample data: the app's demo mode, previews, screenshots and
/// UI tests run on it. Delays are scaled by `latency` so that tests can run at full speed.
public actor MockBackend: RemoteBackend {
    public enum Scenario: String, Sendable, CaseIterable {
        /// Two paired Macs, a mixed inbox and some history.
        case demo
        /// First launch: nothing paired.
        case empty
        /// Paired Macs that are not reachable.
        case offline
        /// The demo, with this month's traffic close to the broker's free quota.
        case quota
        /// The demo with one short question left: sending it empties the inbox.
        case single
        /// The demo with two long questions left: the second must start at its top whatever the first
        /// was scrolled to.
        case longs
        /// The demo with one request left: the Markdown showcase (soft breaks, a table, a diagram, code).
        case markdown
        /// The demo with one request left: a select-only form of three questions that needs a scroll.
        case strict
        /// The demo with one request left: the Markdown showcase's tall message over two short questions.
        case tall
        /// The demo with one request left: a message with four pictures (wide, tall, square, panoramic)
        /// and a note file.
        case gallery
        /// The demo, with both Macs on an AskHuman that predates the todos (`caps.todos == false`).
        case outdated
        /// The demo, with both Macs syncing their todos through iCloud (the Office iMac has a project of its own).
        case bothSync
        /// Paired Macs that have not answered yet, as right after the app opened: the lists are
        /// still loading. They stay so unless a sync delay is given, after which the demo appears.
        case syncing
    }

    /// A submission as the mock received it (for tests).
    public struct Submission: Equatable, Sendable {
        public var macId: String
        public var requestId: String
        public var action: String
        public var answers: [AnswerItem]
        public var wireIndex: Int?
        public var dismiss: Bool
        public var input: String?
    }

    public nonisolated let states: AsyncStream<EngineState>
    public nonisolated let events: AsyncStream<EngineEvent>

    /// A message to an Agent as the mock received it (for tests).
    public struct InterjectRecord: Equatable, Sendable {
        public var macId: String
        public var agentKey: String
        public var text: String
        public var files: Int
    }

    private let stateOut: AsyncStream<EngineState>.Continuation
    let eventOut: AsyncStream<EngineEvent>.Continuation
    var state: EngineState
    private var entries: [HistoryEntry]
    let latency: Double
    /// Real seconds until the `syncing` scenario turns into the demo; `nil` keeps it syncing.
    let syncDelay: Double?
    let now: @Sendable () -> Date
    private var pairingTask: Task<Void, Never>?
    var counter = 0

    // Agents, watches, interjections and tasks (`MockBackend+Agents.swift`).
    var watchTasks: [String: Task<Void, Never>] = [:]
    var interjectQueues: [String: [String]] = [:]
    var taskTasks: [String: Task<Void, Never>] = [:]
    var taskDrafts: [String: TaskDraft] = [:]
    public internal(set) var interjects: [InterjectRecord] = []
    public internal(set) var createdTasks: [TaskDraft] = []
    /// Makes the next new task fail with this reason (a one-shot).
    public var failNextTask: String?
    public var failNextInterject: String?
    /// Whether the Macs list models for the new task form; `false` is a Mac that is too old to.
    public var offersModels = true
    /// The forms behind the Agents the mock started for new tasks (`macId|agentKey`).
    var launchedDrafts: [String: TaskDraft] = [:]
    /// How many Agents one Mac may follow (the real limit unless a test lowers it).
    public var followLimit = Wire.maxFollow
    /// The Agent picks queued messages up after this many seconds (scaled by the latency);
    /// `nil` keeps them queued until `consumeInterjects` is called.
    public var interjectConsumeDelay: Double? = 4

    // Todos (`MockBackend+Todos.swift`).
    var todoRows: [String: [TodoProjectRow]] = [:]
    /// What a Mac sends instead of `todoRows` while it syncs, when it has to say something else.
    var todoSyncedRows: [String: [TodoProjectRow]] = [:]
    /// The todos of each project, by `macId|projectId`.
    var todoLists: [String: ProjectTodos] = [:]
    var todoRev: UInt64 = 1
    var todoWatchers: Set<String> = []
    /// Answers already given, by `clientMsgId`: a repeated write gets the first answer again.
    var todoAcks: [String: TodoAck] = [:]
    public internal(set) var todoOps: [TodoOperation] = []
    /// Makes the next todo write fail the way a Mac that could not store it does (a one-shot).
    public var failNextTodoWrite: String?
    /// What the Home MacBook Pro's sync line says is wrong (`notSignedIn`, `quota`, …); `nil` when nothing is.
    public var todoSyncProblem: String?
    /// The Macs that sync their todos through iCloud (`mode: icloud`); the others keep their own.
    var syncingTodoMacs: Set<String> = [MockData.homeMacId]

    public private(set) var submissions: [Submission] = []
    public private(set) var uploads: [UploadSource] = []
    public private(set) var seen: [String] = []
    /// Mac ids the mock treats as unreachable: submissions to them throw `.macOffline`.
    public var unreachable: Set<String> = []
    /// Makes the next submission fail the way an unanswered one does.
    public var failNextSubmission = false

    /// `syncDelay` (real seconds) ends the `syncing` scenario that long after the start; `cachedInbox`
    /// lets that scenario begin with the requests the phone kept from the last run.
    public init(
        scenario: Scenario = .demo, latency: Double = 1, syncDelay: Double? = nil, cachedInbox: Bool = false,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        let (stateStream, stateContinuation) = AsyncStream<EngineState>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let (eventStream, eventContinuation) = AsyncStream<EngineEvent>.makeStream()
        states = stateStream
        events = eventStream
        stateOut = stateContinuation
        eventOut = eventContinuation
        self.latency = latency
        self.syncDelay = scenario == .syncing ? syncDelay : nil
        self.now = now
        let date = now()
        var initial: EngineState
        switch scenario {
        case .demo, .quota, .single, .longs, .markdown, .strict, .tall, .gallery, .outdated, .bothSync:
            let inbox: [InboxItem]
            switch scenario {
            case .single: inbox = MockData.inbox(now: date).filter { $0.requestId == "req-select" }
            case .longs: inbox = MockData.longPair(now: date)
            case .markdown: inbox = MockData.markdownOnly(now: date)
            case .strict: inbox = MockData.strictOnly(now: date)
            case .tall: inbox = MockData.tallMessageOnly(now: date)
            case .gallery: inbox = MockData.galleryOnly(now: date)
            default: inbox = MockData.inbox(now: date)
            }
            let traffic = scenario == .quota
                ? TrafficStats(month: "2026-10", sentBytes: 230_000_000, receivedBytes: 240_000_000, sentMessages: 41_000, receivedMessages: 77_000)
                : TrafficStats(month: "2026-10", sentBytes: 18_432, receivedBytes: 96_210, sentMessages: 41, receivedMessages: 77)
            initial = EngineState(
                phase: .connected(via: "tcp"), foreground: true,
                macs: MockData.macs(now: date, knowTodos: scenario != .outdated), inbox: inbox,
                push: .registered(expiresAt: date.addingTimeInterval(86_400 * 80)), historyRevision: 1,
                traffic: traffic,
                brokerHost: "broker.example.com", agents: MockData.agents(now: date)
            )
            entries = MockData.history(now: date)
        case .empty:
            initial = EngineState(phase: .idle, foreground: true, push: .noToken)
            entries = []
        case .offline:
            initial = EngineState(
                phase: .backoff(retryAt: date.addingTimeInterval(20), reason: "network"), foreground: true,
                macs: MockData.macs(now: date, link: .brokerDown), inbox: MockData.inbox(now: date),
                push: .registered(expiresAt: date.addingTimeInterval(86_400 * 80)), historyRevision: 1, brokerHost: "broker.example.com"
            )
            entries = MockData.history(now: date)
        case .syncing:
            initial = EngineState(
                phase: .connecting(via: "tcp"), foreground: true,
                macs: MockData.macs(now: date, link: .connecting), inbox: cachedInbox ? MockData.inbox(now: date) : [],
                push: .registered(expiresAt: date.addingTimeInterval(86_400 * 80)), historyRevision: 1, brokerHost: "broker.example.com"
            )
            entries = MockData.history(now: date)
        }
        for index in initial.macs.indices {
            initial.macs[index].pendingCount = initial.inbox.filter { $0.macId == initial.macs[index].id }.count
        }
        state = initial
        let sample = MockData.todoSample(now: date)
        todoRows = sample.rows
        todoSyncedRows = sample.syncedRows
        todoLists = sample.lists
        if scenario == .bothSync { syncingTodoMacs = [MockData.homeMacId, MockData.officeMacId] }
    }

    // MARK: lifecycle

    public func start() async {
        publish()
        if let syncDelay {
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(syncDelay))
                await self?.finishSync()
            }
        }
    }

    /// Ends the `syncing` scenario: the broker link is up, the Macs answered and the lists fill in.
    public func finishSync() {
        let date = now()
        state.phase = .connected(via: "tcp")
        state.macs = MockData.macs(now: date)
        if state.inbox.isEmpty { state.inbox = MockData.inbox(now: date) }
        state.agents = MockData.agents(now: date)
        state.traffic = TrafficStats(month: "2026-10", sentBytes: 18_432, receivedBytes: 96_210, sentMessages: 41, receivedMessages: 77)
        refreshCounts()
        publish()
    }

    public func setForeground(_ foreground: Bool) async {
        state.foreground = foreground
        publish()
    }

    /// The mock has no connection to keep: being inactive changes nothing.
    public func setInactive() async {}

    public func refresh() async {
        await pause(0.6)
        publish()
    }

    /// Mac ids the app asked to look into the link of (a notification arrived for a request the inbox lacks).
    public private(set) var linkChecks: [String] = []

    /// The mock has no link to repair: it only keeps the Mac ids for tests to see.
    public func checkLink(macId: String) async {
        linkChecks.append(macId)
    }

    public private(set) var deviceName = "iPhone"

    public func setDeviceName(_ name: String) async {
        deviceName = name
    }

    // MARK: pairing

    public func startPairing(code: String) async {
        pairingTask?.cancel()
        let expires = now().addingTimeInterval(300)
        guard code.hasPrefix("ahpair1.") else {
            state.pairing = PairingState(stage: .failed(.invalidCode), macName: "", sas: nil, expiresAt: expires)
            publish()
            return
        }
        state.pairing = PairingState(stage: .connecting, macName: "Travel MacBook Air", sas: nil, expiresAt: expires)
        publish()
        pairingTask = Task { [weak self] in
            guard let self else { return }
            await self.runPairing(expires: expires)
        }
    }

    private func runPairing(expires: Date) async {
        let name = "Travel MacBook Air"
        for (stage, sas, delay) in [
            (PairingStage.waitingForMac, String?.none, 0.8),
            (PairingStage.macConfirming, "428 913", 2.4),
            (PairingStage.finishing, "428 913", 0.8),
        ] {
            await pause(delay)
            if Task.isCancelled { return }
            state.pairing = PairingState(stage: stage, macName: name, sas: sas, expiresAt: expires)
            publish()
        }
        await pause(0.8)
        if Task.isCancelled { return }
        let macId = "mocktravelmacmacmacmacmac"
        state.macs.append(MacState(
            id: macId, name: name, macName: name, alias: nil, link: .online,
            caps: Caps(newTask: true, interject: true, watch: true), slot: SlotInfo(active: false, autoActivation: true),
            pairedAt: now(), lastSeenAt: now(), pendingCount: 0, pushStatus: "ok"
        ))
        state.brokerHost = state.brokerHost ?? "broker.example.com"
        state.pairing = PairingState(stage: .done(macId: macId), macName: name, sas: "428 913", expiresAt: expires)
        publish()
        eventOut.yield(.pairingFinished(macId: macId))
    }

    public func cancelPairing() async {
        pairingTask?.cancel()
        state.pairing = nil
        publish()
    }

    public func finishPairing() async {
        state.pairing = nil
        publish()
    }

    public func unpair(macId: String) async {
        guard let mac = state.macs.first(where: { $0.id == macId }) else { return }
        state.macs.removeAll { $0.id == macId }
        state.inbox.removeAll { $0.macId == macId }
        dropAgents(of: macId)
        refreshCounts()
        publish()
        eventOut.yield(.macUnpaired(macId: macId, macName: mac.name))
    }

    public func forgetAll() async {
        pairingTask?.cancel()
        for mac in state.macs {
            eventOut.yield(.macUnpaired(macId: mac.id, macName: mac.name))
            dropAgents(of: mac.id)
        }
        state.macs = []
        state.inbox = []
        state.agents = []
        state.tasks = []
        state.pairing = nil
        state.brokerHost = nil
        state.phase = .idle
        publish()
    }

    public func renameMac(macId: String, alias: String?) async {
        guard let index = state.macs.firstIndex(where: { $0.id == macId }) else { return }
        let trimmed = alias?.trimmingCharacters(in: .whitespacesAndNewlines)
        state.macs[index].alias = (trimmed?.isEmpty ?? true) ? nil : trimmed
        state.macs[index].name = state.macs[index].alias ?? state.macs[index].macName
        for item in state.inbox.indices where state.inbox[item].macId == macId { state.inbox[item].macName = state.macs[index].name }
        publish()
    }

    // MARK: requests

    public func markSeen(macId: String, requestId: String) async {
        guard let index = state.inbox.firstIndex(where: { $0.macId == macId && $0.requestId == requestId }) else { return }
        seen.append(requestId)
        state.inbox[index].unread = false
        publish()
    }

    public func submitAnswer(macId: String, requestId: String, action: String, answers: [AnswerItem]) async throws -> String {
        let index = try inboxIndex(macId: macId, requestId: requestId, kind: "ask")
        submissions.append(Submission(macId: macId, requestId: requestId, action: action, answers: answers, wireIndex: nil, dismiss: false, input: nil))
        return try await settle(index: index, outcome: action == "cancel" ? .cancelled : .answered, answers: answers, choice: nil, input: nil)
    }

    public func submitConfirm(macId: String, requestId: String, wireIndex: Int?, dismiss: Bool, input: String?) async throws -> String {
        let index = try inboxIndex(macId: macId, requestId: requestId, kind: "confirm")
        submissions.append(Submission(macId: macId, requestId: requestId, action: "decide", answers: [], wireIndex: wireIndex, dismiss: dismiss, input: input))
        let label = state.inbox[index].view?.confirm?.choices.first { $0.wireIndex == wireIndex }?.label
        return try await settle(index: index, outcome: dismiss ? .dismissed : .answered, answers: [], choice: label, input: input)
    }

    private func inboxIndex(macId: String, requestId: String, kind: String) throws -> Int {
        guard let index = state.inbox.firstIndex(where: { $0.macId == macId && $0.requestId == requestId }),
              state.inbox[index].view != nil, state.inbox[index].summary.kind == kind
        else { throw BackendError.unknownRequest }
        if unreachable.contains(macId) || state.macs.first(where: { $0.id == macId })?.link != .online { throw BackendError.macOffline }
        if case .sending = state.inbox[index].state { throw BackendError.invalid("This request is already being sent.") }
        return index
    }

    private func settle(index: Int, outcome: HistoryOutcome, answers: [AnswerItem], choice: String?, input: String?) async throws -> String {
        let item = state.inbox[index]
        counter += 1
        let clientMsgId = "mock-msg-\(counter)"
        if failNextSubmission {
            failNextSubmission = false
            state.inbox[index].state = .failed(clientMsgId: clientMsgId, reason: "timeout")
            publish()
            return clientMsgId
        }
        state.inbox[index].state = .sending(clientMsgId: clientMsgId, since: now())
        publish()
        // The Mac acknowledges after a moment; the request then moves into the history.
        let delay = latency * 0.7
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            await self?.finish(item.id, outcome: outcome, answers: answers, choice: choice, input: input)
        }
        return clientMsgId
    }

    /// `by` names the other phone that finished the request; it is `nil` when our own submission did.
    private func finish(
        _ id: String, outcome: HistoryOutcome, answers: [AnswerItem], choice: String?, input: String?, by: String? = nil
    ) {
        guard let index = state.inbox.firstIndex(where: { $0.id == id }), let view = state.inbox[index].view else { return }
        let item = state.inbox.remove(at: index)
        entries.insert(
            HistoryEntry(
                id: item.id, macId: item.macId, macName: item.macName, request: view, outcome: outcome, answers: answers,
                confirmChoice: choice, confirmInput: input, receivedAt: item.receivedAt, finishedAt: now()
            ),
            at: 0
        )
        state.historyRevision += 1
        refreshCounts()
        publish()
        eventOut.yield(.requestFinished(macId: item.macId, requestId: item.requestId, outcome: outcome, by: by))
    }

    public func retrySubmission(clientMsgId: String) async throws {
        guard let index = state.inbox.firstIndex(where: {
            if case .failed(let id, _) = $0.state { return id == clientMsgId }
            return false
        }) else { throw BackendError.unknownRequest }
        let item = state.inbox[index]
        state.inbox[index].state = .sending(clientMsgId: clientMsgId, since: now())
        publish()
        let delay = latency * 0.7
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            await self?.finish(item.id, outcome: .answered, answers: [], choice: nil, input: nil)
        }
    }

    public func discardSubmission(clientMsgId: String) async {
        for index in state.inbox.indices {
            switch state.inbox[index].state {
            case .failed(let id, _) where id == clientMsgId, .sending(let id, _) where id == clientMsgId:
                state.inbox[index].state = .pending
            default: break
            }
        }
        publish()
    }

    public func upload(
        macId: String, requestId: String, source: UploadSource, progress: @escaping @Sendable (Double) -> Void
    ) async throws -> String {
        guard !unreachable.contains(macId) else { throw BackendError.macOffline }
        uploads.append(source)
        for step in 1...4 {
            await pause(0.12)
            progress(Double(step) / 4)
        }
        counter += 1
        return "mock-upload-\(counter)"
    }

    public func download(macId: String, asset: AssetRef, progress: @escaping @Sendable (Double) -> Void) async throws -> URL {
        for step in 1...3 {
            await pause(0.15)
            progress(Double(step) / 3)
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ah-mock-assets", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent(asset.assetId + "-" + asset.name)
        try MockData.contents(of: asset).write(to: url)
        return url
    }

    // MARK: history

    public func history() async -> [HistoryEntry] { entries }

    public func deleteHistory(id: String) async {
        entries.removeAll { $0.id == id }
        state.historyRevision += 1
        publish()
    }

    public func clearHistory() async {
        entries.removeAll()
        state.historyRevision += 1
        publish()
    }

    public func updatePushToken(_ token: Data?) async {
        state.push = token == nil ? .noToken : .registered(expiresAt: now().addingTimeInterval(86_400 * 90))
        publish()
    }

    // MARK: demo controls

    /// Ends a request as if another phone had answered it: it leaves the inbox and the finish event
    /// names who did it. `submitAnswer` cannot model this, because the end of our own submission
    /// carries no name and makes the app move on to the next request.
    public func simulateFinishedElsewhere(requestId: String, macId: String = MockData.homeMacId, by: String = "Another phone") {
        guard let item = state.inbox.first(where: { $0.macId == macId && $0.requestId == requestId }) else { return }
        finish(item.id, outcome: .answered, answers: [], choice: nil, input: nil, by: by)
    }

    /// Delivers a new request, as if the Agent on `macId` had just asked (from a session called
    /// `sessionTitle`, when given).
    public func simulateArrival(macId: String = MockData.homeMacId, sessionTitle: String? = nil) {
        counter += 1
        let date = now()
        let id = "req-live-\(counter)"
        let view = RequestView(
            id: id, kind: "ask", createdAt: MockData.ms(date), agent: AgentRef(kind: "cursor", label: "Cursor"),
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: sessionTitle,
            source: "cursor", contentHash: "h-" + id,
            ask: AskView(
                isMarkdown: true,
                message: MessageView(text: "测试已通过，需要我提交这些改动吗？", files: []),
                questions: [QuestionView(text: "下一步", options: [
                    OptionView(text: "提交并推送", recommended: true), OptionView(text: "只提交，不推送", recommended: false),
                ])],
                selectOnly: false, single: true
            )
        )
        let name = state.macs.first { $0.id == macId }?.name ?? "Mac"
        state.inbox.insert(
            InboxItem(
                macId: macId, macName: name, requestId: id,
                summary: RequestSummary(
                    id: id, kind: "ask", project: view.project, agent: view.agent, sessionTitle: sessionTitle,
                    createdAt: view.createdAt, preview: MockData.preview(of: view), contentHash: view.contentHash, size: 1_024
                ),
                view: view, receivedAt: date, unread: true, state: .pending
            ),
            at: 0
        )
        refreshCounts()
        publish()
        eventOut.yield(.requestArrived(macId: macId, requestId: id))
    }

    public func setUnreachable(_ macIds: Set<String>) {
        unreachable = macIds
    }

    public func setFailNextSubmission(_ fail: Bool) {
        failNextSubmission = fail
    }

    // MARK: helpers

    func pause(_ seconds: Double) async {
        let scaled = seconds * latency
        if scaled > 0 { try? await Task.sleep(for: .seconds(scaled)) }
    }

    func refreshCounts() {
        for index in state.macs.indices {
            state.macs[index].pendingCount = state.inbox.filter { $0.macId == state.macs[index].id }.count
        }
    }

    func publish() {
        stateOut.yield(state)
    }
}
