import Foundation
import AHProtocol
import AHStore

/// Which Agents the Agents tab lists.
public enum AgentFilter: Hashable, Sendable {
    case all
    case mac(String)
}

/// Agents of one state, in the order the tab shows them (questions first).
public struct AgentSection: Identifiable, Equatable, Sendable {
    /// `waiting`, `working`, `idle` or `ended`.
    public var id: String
    public var items: [AgentItem]
}

extension AppStore {
    // MARK: list

    public func agentItems(filter: AgentFilter? = nil) -> [AgentItem] {
        let active = filter ?? agentFilter
        return state.agents.filter { item in
            switch active {
            case .all: return true
            case .mac(let id): return item.macId == id
            }
        }
    }

    public func agentSections(filter: AgentFilter? = nil) -> [AgentSection] {
        let items = agentItems(filter: filter)
        func bucket(_ item: AgentItem) -> String {
            if item.needsAttention { return "attention" }
            return ["waiting", "working", "ended"].contains(item.view.state) ? item.view.state : "idle"
        }
        return ["waiting", "attention", "working", "idle", "ended"].compactMap { id in
            let group = items.filter { bucket($0) == id }
            return group.isEmpty ? nil : AgentSection(id: id, items: group)
        }
    }

    public func agent(id: String) -> AgentItem? {
        state.agents.first { $0.id == id }
    }

    /// The Agent of a page: the live one, or the last known version of one that is gone.
    public func displayAgent(id: String) -> AgentItem? {
        agent(id: id) ?? retainedAgents[id]?.item
    }

    public func watchState(id: String) -> WatchState? {
        state.watches[id]
    }

    public var waitingAgentCount: Int { state.agents.filter { $0.view.state == "waiting" }.count }

    /// Macs a task can be started on (online and told the phone may).
    public var taskMacs: [MacState] {
        state.macs.filter { $0.link == .online && $0.caps.newTask }
    }

    // MARK: opening

    public func openAgent(macId: String, agentKey: String) {
        tab = .agents
        let id = AgentItem.key(macId, agentKey)
        guard agentPath.last != id else { return }
        agentPath = [id]
    }

    /// Opens what a tapped notification is about: the Agent of a follow notification (`follow.*`,
    /// whose `rid` is the Agent key), else the request. `hint` is what the notification said; the
    /// request page shows it until the request itself has reached the phone.
    public func openNotification(macId: String, rid: String, kind: String?, hint: NotificationHint? = nil) {
        if kind?.hasPrefix("follow.") == true {
            openAgent(macId: macId, agentKey: rid)
        } else {
            if let hint { hints[CachedRequest.key(macId, rid)] = hint }
            open(macId: macId, requestId: rid)
        }
    }

    /// A notification that arrives while the app is open is redundant when the app already shows
    /// what it is about (the in-app banner covers new requests and follow events).
    public func coversNotification(macId: String, rid: String, kind: String?) -> Bool {
        guard state.foreground else { return false }
        if kind?.hasPrefix("follow.") == true {
            return state.agent(macId: macId, agentKey: rid) != nil
        }
        return item(id: macId + "/" + rid) != nil
    }

    /// A notification arrived while the app is open for a request the phone does not hold. A Mac pushes
    /// a request only when the phone did not acknowledge it in time, so the phone's link or session to
    /// that Mac is broken: have the engine look into it now (`RemoteBackend.checkLink`) rather than
    /// leave the inbox empty until the user pulls it down. Not for the news that a request was handled
    /// (`resolved`), nor for an Agent's (`follow.*`): neither says anything about the inbox.
    public func noteUnknownRequest(macId: String, rid: String, kind: String?) {
        guard state.foreground, let kind, kind != "resolved", !kind.hasPrefix("follow.") else { return }
        // A request that was here a moment ago (it ended, or was answered on this phone) is not missing.
        guard displayItem(id: CachedRequest.key(macId, rid)) == nil else { return }
        let backend = backend
        Task { await backend.checkLink(macId: macId) }
    }

    /// Opens or closes the live frames of the Agent a page shows.
    func setWatch(_ id: String, open: Bool) {
        guard let (macId, agentKey) = AgentItem.split(id) else { return }
        let backend = backend
        Task { await backend.watch(macId: macId, agentKey: agentKey, open: open) }
    }

    func presentFollowEvent(macId: String, event: FollowEvent) {
        guard state.foreground else { return }
        let id = AgentItem.key(macId, event.agentKey)
        // The page of that Agent shows it already.
        if tab == .agents, agentPath.last == id { return }
        arrivalTick += 1
        show(Banner(
            kind: .agent(macId: macId, agentKey: event.agentKey, event: event.kind),
            title: event.title.isEmpty ? event.project : event.title,
            subtitle: event.preview.isEmpty ? event.project : event.preview
        ))
    }

    // MARK: actions

    /// Follows or unfollows an Agent: the Mac tells the phone about its first result (`done` or `failed`).
    /// The app has no button for it (the Macs follow the tasks the phone starts); UI test runs ask for
    /// a follow at launch. Returns `false` when the Mac refused or could not be reached.
    @discardableResult
    public func setFollow(_ item: AgentItem, on: Bool) async -> Bool {
        do {
            try await backend.setFollow(macId: item.macId, agentKey: item.agentKey, on: on)
            return true
        } catch BackendError.rejected(let reason) where reason == "limit" {
            // The Mac's follow list is full: no one is waiting for an answer, the caller gets `false`.
        } catch {
            show(Banner(kind: .warning, title: item.macName, subtitle: error.localizedDescription))
        }
        return false
    }

    public func interject(
        _ item: AgentItem, text: String, files: [UploadSource], clientMsgId: String?, progress: @escaping @Sendable (Double) -> Void
    ) async throws -> InterjectOutcome {
        try await backend.sendInterject(
            macId: item.macId, agentKey: item.agentKey, text: text, files: files, clientMsgId: clientMsgId, progress: progress
        )
    }

    /// The message form of an Agent's page (kept while the page may still be open).
    public func interjectComposer(for item: AgentItem) -> InterjectComposer {
        if let existing = interjectComposers[item.id] { return existing }
        let composer = InterjectComposer(key: item.id)
        interjectComposers[item.id] = composer
        return composer
    }

    /// Uploads the attachments and sends the message; the form is emptied once the Mac took it.
    @discardableResult
    public func send(_ composer: InterjectComposer, to item: AgentItem) async throws -> InterjectOutcome {
        if needsAgentDeliveryReview(item) { throw BackendError.rejected("controlUnknown") }
        if composer.isSending || continuation(for: item).isSending { throw BackendError.rejected("controlStarting") }
        composer.isSending = true
        composer.uploadProgress = nil
        defer {
            composer.isSending = false
            composer.uploadProgress = nil
        }
        // A send whose outcome is unknown is repeated under the same id: the Mac answers it from its journal.
        let id = composer.pendingMessageId ?? Bytes.newMsgId()
        composer.pendingMessageId = id
        do {
            let outcome = try await backend.sendInterject(
                macId: item.macId, agentKey: item.agentKey, text: composer.trimmedText, files: try composer.sources(), clientMsgId: id,
                progress: { fraction in Task { @MainActor in composer.uploadProgress = fraction } }
            )
            composer.reset()
            noteNotificationLimit(outcome, item: item)
            return outcome
        } catch {
            // Unknown delivery must keep its identity: a new one could submit the message twice.
            if case BackendError.rejected(let reason) = error {
                if ["desktopUnknown", "controlUnknown", "interrupted", "conflict"].contains(reason) {
                    composer.needsDeliveryReview = true
                } else {
                    composer.pendingMessageId = nil
                }
            }
            throw error
        }
    }

    public func needsAgentDeliveryReview(_ item: AgentItem) -> Bool {
        interjectComposer(for: item).needsDeliveryReview || continuation(for: item).needsDeliveryReview
    }

    public func confirmAgentDeliveryReviewed(_ item: AgentItem) {
        interjectComposer(for: item).confirmDeliveryReviewed()
        continuation(for: item).confirmDeliveryReviewed()
    }

    public func continuation(for item: AgentItem) -> AgentContinuation {
        if let existing = continuations[item.id] { return existing }
        let continuation = AgentContinuation()
        continuations[item.id] = continuation
        return continuation
    }

    public func continueAgent(_ item: AgentItem) async throws -> InterjectOutcome {
        let action = continuation(for: item)
        guard !action.isSending, !interjectComposer(for: item).isSending else { throw BackendError.rejected("controlStarting") }
        if needsAgentDeliveryReview(item) { throw BackendError.rejected("controlUnknown") }
        guard let current = agent(id: item.id), current.needsAttention, current.view.canInterject.ok,
              current.view.state == "idle", let resultId = current.view.lastTurnResult?.id, !resultId.isEmpty,
              action.submittedResultId != resultId
        else { throw BackendError.rejected("resultChanged") }
        if action.needsDeliveryReview { throw BackendError.rejected("controlUnknown") }
        if action.resultId != resultId { action.messageId = nil; action.resultId = resultId }
        let messageId = action.messageId ?? Bytes.newMsgId()
        action.messageId = messageId
        action.isSending = true
        defer { action.isSending = false }
        do {
            let outcome = try await backend.continueAgent(macId: item.macId, agentKey: item.agentKey, expectedResultId: resultId, clientMsgId: messageId)
            action.submittedResultId = resultId
            action.messageId = nil
            noteNotificationLimit(outcome, item: item)
            return outcome
        } catch {
            if case BackendError.rejected(let reason) = error {
                if ["controlUnknown", "desktopUnknown", "interrupted", "conflict"].contains(reason) { action.needsDeliveryReview = true }
                else { action.messageId = nil }
            }
            throw error
        }
    }

    private func noteNotificationLimit(_ outcome: InterjectOutcome, item: AgentItem) {
        if outcome.notificationRegistered == false {
            show(Banner(kind: .warning, title: item.macName, subtitle: String(localized: "Message sent. The notification limit has been reached.")))
        }
    }

    /// Withdraws the messages still waiting for an Agent.
    public func clearInterject(_ item: AgentItem) async throws {
        _ = try await backend.clearInterject(macId: item.macId, agentKey: item.agentKey)
    }

    /// Takes the waiting messages back: returns their text (to edit again) and withdraws them.
    public func recallInterject(_ item: AgentItem) async throws -> String {
        let info = try await backend.queryInterject(macId: item.macId, agentKey: item.agentKey)
        _ = try await backend.clearInterject(macId: item.macId, agentKey: item.agentKey)
        return info.text ?? ""
    }

    // MARK: new tasks

    /// Opens the new task form, on `macId` or the first Mac that can start tasks.
    public func presentNewTask(macId: String? = nil) {
        guard let target = macId ?? taskMacs.first?.id else { return }
        let composer = TaskComposer(macId: target)
        taskComposer = composer
        let backend = backend
        Task { await backend.loadTaskOptions(macId: target) }
    }

    /// Closes the new task sheet. A task that was sent keeps going.
    public func dismissNewTask() {
        taskComposer = nil
        viewingTask = nil
    }

    /// Shows how a task that was sent is doing.
    public func presentTaskProgress(_ id: String) {
        viewingTask = id
    }

    /// Whether the new task sheet is up.
    public var isTaskSheetPresented: Bool { taskComposer != nil || viewingTask != nil }

    /// Tasks still on their way to an Agent, or that never got there: the Agents tab lists them.
    public var openTasks: [TaskProgress] {
        state.tasks.filter { !$0.foundAgent }
    }

    public func selectTaskMac(_ macId: String) {
        guard let composer = taskComposer, composer.macId != macId else { return }
        composer.switchMac(to: macId)
        let backend = backend
        Task { await backend.loadTaskOptions(macId: macId) }
    }

    public func reloadTaskOptions() {
        guard let macId = taskComposer?.macId else { return }
        let backend = backend
        Task { await backend.loadTaskOptions(macId: macId) }
    }

    /// Picks an Agent and restores this phone's submitted model and effort preference for it.
    public func selectTaskAgent(_ kind: String) {
        taskComposer?.selectAgent(kind)
        taskOptionsArrived()
    }

    /// Chooses a workspace; its TODOs are asked for.
    public func selectTaskWorkspace(_ workspaceId: String) {
        guard let composer = taskComposer else { return }
        composer.workspaceId = workspaceId
        composer.todoId = nil
        let macId = composer.macId
        let backend = backend
        Task { await backend.loadTaskTodos(macId: macId, workspaceId: workspaceId) }
    }

    /// Fills in the form's defaults once the choices of its Mac are there.
    public func taskOptionsArrived() {
        guard let composer = taskComposer, case .ready(let options)? = state.taskOptions[composer.macId] else { return }
        let before = composer.workspaceId
        composer.applyDefaults(options: options, defaults: defaults)
        if let workspaceId = composer.workspaceId, workspaceId != before, state.todos(macId: composer.macId, workspaceId: workspaceId) == nil {
            let macId = composer.macId
            let backend = backend
            Task { await backend.loadTaskTodos(macId: macId, workspaceId: workspaceId) }
        }
    }

    /// Sends the form. Returns the id of the task to watch, or `nil` when the form is incomplete.
    public func createTask() async throws -> String? {
        guard let composer = taskComposer, case .ready(let options)? = state.taskOptions[composer.macId],
              let draft = composer.draft(options: options)
        else { return nil }
        composer.remember(options: options, defaults: defaults)
        return try await backend.createTask(macId: composer.macId, draft: draft)
    }

    public func task(id: String) -> TaskProgress? {
        state.tasks.first { $0.id == id }
    }

    public func retryTask(_ id: String) async throws {
        try await backend.retryTask(id: id)
    }

    public func dismissTask(_ id: String) async {
        await backend.dismissTask(id: id)
    }

    public func takeSlot(_ mac: MacState) async throws {
        try await backend.takeSlot(macId: mac.id)
    }
}
