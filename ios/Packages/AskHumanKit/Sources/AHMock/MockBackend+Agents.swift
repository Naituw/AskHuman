import Foundation
import AHProtocol
import AHCore

// Agents, live watching, interjections and new tasks of the mock backend. The timing mimics a real
// Mac: frames every two seconds, a message that the Agent reads a few seconds after it was sent, a
// task that goes from accepted to watching in about two seconds.

extension MockBackend {
    private func agentIndex(_ macId: String, _ agentKey: String) -> Int? {
        state.agents.firstIndex { $0.macId == macId && $0.agentKey == agentKey }
    }

    private func requireOnline(_ macId: String) throws {
        if unreachable.contains(macId) || state.macs.first(where: { $0.id == macId })?.link != .online {
            throw BackendError.macOffline
        }
    }

    /// Forgets everything about the Agents of a Mac that was unpaired.
    func dropAgents(of macId: String) {
        state.agents.removeAll { $0.macId == macId }
        for key in Array(state.watches.keys) where key.hasPrefix(macId + "|") {
            watchTasks[key]?.cancel()
            watchTasks[key] = nil
            state.watches[key] = nil
        }
        for task in state.tasks where task.macId == macId {
            taskTasks[task.id]?.cancel()
            taskTasks[task.id] = nil
        }
        state.tasks.removeAll { $0.macId == macId }
        state.taskOptions[macId] = nil
    }

    // MARK: follow

    public func setFollow(macId: String, agentKey: String, on: Bool) async throws {
        try requireOnline(macId)
        guard let index = agentIndex(macId, agentKey) else { throw BackendError.rejected("notFound") }
        let followed = state.agents.filter { $0.macId == macId && $0.view.followed }.count
        if on, !state.agents[index].view.followed, followed >= followLimit { throw BackendError.rejected("limit") }
        state.agents[index].view.followed = on
        publish()
    }

    // MARK: watch

    public func watch(macId: String, agentKey: String, open: Bool) async {
        let id = AgentItem.key(macId, agentKey)
        watchTasks[id]?.cancel()
        watchTasks[id] = nil
        guard open else {
            state.watches[id] = nil
            publish()
            return
        }
        guard let item = state.agents.first(where: { $0.id == id }), state.macs.first(where: { $0.id == macId })?.link == .online else {
            state.watches[id] = WatchState(macId: macId, agentKey: agentKey, closed: "notFound", receivedAt: now())
            publish()
            return
        }
        state.watches[id] = WatchState(
            macId: macId, agentKey: agentKey, seq: 1, frame: frame(of: item, tick: 0), receivedAt: now()
        )
        publish()
        guard item.view.state == "working" else { return }
        let interval = max(2 * latency, 0.05)
        watchTasks[id] = Task { [weak self] in
            for tick in 1...6 {
                try? await Task.sleep(for: .seconds(interval))
                if Task.isCancelled { return }
                await self?.advanceWatch(id, tick: tick)
            }
        }
    }

    /// The frame of an Agent. One that was started from the phone runs with what was picked there
    /// (and with nothing the Mac knows of when the pick was "Default").
    private func frame(of item: AgentItem, tick: Int) -> WatchFrameData {
        var frame = MockData.watchFrame(of: item.view, tick: tick, now: now())
        if let draft = launchedDrafts[item.id] {
            frame.model = draft.modelLabel
            frame.reasoning = draft.effort
        }
        return frame
    }

    func advanceWatch(_ id: String, tick: Int) {
        guard var watch = state.watches[id], let item = state.agents.first(where: { $0.id == id }) else { return }
        watch.seq += 1
        watch.frame = frame(of: item, tick: tick)
        watch.receivedAt = now()
        state.watches[id] = watch
        publish()
    }

    // MARK: interjections

    public func sendInterject(
        macId: String, agentKey: String, text: String, files: [UploadSource], clientMsgId: String?,
        progress: @escaping @Sendable (Double) -> Void
    ) async throws -> InterjectOutcome {
        try requireOnline(macId)
        if let reason = failNextInterject {
            failNextInterject = nil
            throw BackendError.rejected(reason)
        }
        guard let first = agentIndex(macId, agentKey) else { throw BackendError.rejected("notFound") }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.count > 20_000 { throw BackendError.rejected("tooLong") }
        if trimmed.isEmpty && files.isEmpty { throw BackendError.rejected("empty") }
        let view = state.agents[first].view
        guard view.canInterject.ok else { throw BackendError.rejected(view.canInterject.reason ?? "notWorking") }
        for index in files.indices {
            for step in 1...3 {
                await pause(0.08)
                progress((Double(index) + Double(step) / 3) / Double(files.count))
            }
        }
        // The Agent may have changed while the files went over.
        guard let index = agentIndex(macId, agentKey), state.agents[index].view.canInterject.ok else {
            throw BackendError.rejected("notWorking")
        }
        interjects.append(InterjectRecord(macId: macId, agentKey: agentKey, text: trimmed, files: files.count))
        if state.agents[index].view.state == "idle" {
            state.agents[index].view.state = "working"
            state.agents[index].view.lastTurnResult = nil
            state.agents[index].view.lastTurnFailed = false
            state.agents[index].view.followed = true
            publish()
            return InterjectOutcome(status: "delivered", pending: 0, notificationRegistered: true)
        }

        let key = AgentItem.key(macId, agentKey)
        interjectQueues[key, default: []].append(trimmed)
        let pending = interjectQueues[key]?.count ?? 1
        state.agents[index].view.pendingInterject = UInt64(pending)
        publish()
        if let delay = interjectConsumeDelay {
            let seconds = delay * latency
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(seconds))
                await self?.consumeInterjects(macId: macId, agentKey: agentKey)
            }
        }
        return InterjectOutcome(status: "queued", pending: pending)
    }

    public func continueAgent(macId: String, agentKey: String, expectedResultId: String, clientMsgId: String) async throws -> InterjectOutcome {
        try requireOnline(macId)
        if let reason = failNextInterject { failNextInterject = nil; throw BackendError.rejected(reason) }
        guard let index = agentIndex(macId, agentKey), state.agents[index].view.state == "idle",
              state.agents[index].view.canInterject.ok, state.agents[index].view.lastTurnResult?.id == expectedResultId,
              state.agents[index].needsAttention else { throw BackendError.rejected("resultChanged") }
        interjects.append(InterjectRecord(macId: macId, agentKey: agentKey, text: "Inspect the previous attempt and continue from current progress.", files: 0))
        let registered = state.agents.filter { $0.macId == macId && $0.view.followed }.count < followLimit || state.agents[index].view.followed
        state.agents[index].view.state = "working"
        state.agents[index].view.lastTurnResult = nil
        state.agents[index].view.lastTurnFailed = false
        state.agents[index].view.followed = registered
        publish()
        return InterjectOutcome(status: "delivered", pending: 0, notificationRegistered: registered)
    }

    /// The Agent reads the messages that were waiting for it.
    public func consumeInterjects(macId: String, agentKey: String) {
        let key = AgentItem.key(macId, agentKey)
        guard let queue = interjectQueues[key], !queue.isEmpty else { return }
        interjectQueues[key] = nil
        if let index = agentIndex(macId, agentKey) { state.agents[index].view.pendingInterject = 0 }
        publish()
        eventOut.yield(.interjectConsumed(macId: macId, agentKey: agentKey))
    }

    public func clearInterject(macId: String, agentKey: String) async throws -> Int {
        try requireOnline(macId)
        interjectQueues[AgentItem.key(macId, agentKey)] = nil
        if let index = agentIndex(macId, agentKey) { state.agents[index].view.pendingInterject = 0 }
        publish()
        return 0
    }

    public func queryInterject(macId: String, agentKey: String) async throws -> InterjectInfo {
        try requireOnline(macId)
        let queue = interjectQueues[AgentItem.key(macId, agentKey)] ?? []
        return InterjectInfo(pending: queue.count, text: queue.joined(separator: "\n\n"))
    }

    // MARK: tasks

    public func loadTaskOptions(macId: String) async {
        guard state.macs.first(where: { $0.id == macId })?.link == .online, !unreachable.contains(macId) else {
            state.taskOptions[macId] = .failed("offline")
            publish()
            return
        }
        state.taskOptions[macId] = .loading
        publish()
        await pause(0.5)
        state.taskOptions[macId] = .ready(MockData.taskOptions(macId: macId, models: offersModels))
        publish()
    }

    public func loadTaskTodos(macId: String, workspaceId: String) async {
        await pause(0.3)
        state.taskTodos[EngineState.todosKey(macId: macId, workspaceId: workspaceId)] = MockData.todos(workspaceId: workspaceId)
        publish()
    }

    public func createTask(macId: String, draft: TaskDraft) async throws -> String {
        try requireOnline(macId)
        counter += 1
        let id = "mock-task-\(counter)"
        createdTasks.append(draft)
        taskDrafts[id] = draft
        state.tasks.insert(
            TaskProgress(
                id: id, macId: macId, workspace: draft.workspaceName, agentLabel: draft.agentLabel, text: draft.text,
                stage: "sending", createdAt: now(), modelLabel: draft.modelLabel, effort: draft.effort
            ),
            at: 0
        )
        publish()
        let failure = failNextTask
        failNextTask = nil
        runTask(id, draft: draft, failure: failure)
        return id
    }

    private func runTask(_ id: String, draft: TaskDraft, failure: String?) {
        let step = latency * 0.5
        taskTasks[id] = Task { [weak self] in
            try? await Task.sleep(for: .seconds(step * 0.6))
            await self?.advanceTask(id, stage: "accepted")
            if let failure {
                try? await Task.sleep(for: .seconds(step))
                await self?.advanceTask(id, stage: "failed", reason: failure)
                return
            }
            try? await Task.sleep(for: .seconds(step))
            await self?.advanceTask(id, stage: "launching")
            try? await Task.sleep(for: .seconds(step))
            await self?.advanceTask(id, stage: "launched")
            try? await Task.sleep(for: .seconds(step))
            await self?.matchTask(id, draft: draft)
        }
    }

    func advanceTask(_ id: String, stage: String, reason: String? = nil) {
        guard let index = state.tasks.firstIndex(where: { $0.id == id }) else { return }
        state.tasks[index].stage = stage
        state.tasks[index].reason = reason
        publish()
    }

    /// The Agent session of the task shows up and is followed.
    func matchTask(_ id: String, draft: TaskDraft) {
        guard let index = state.tasks.firstIndex(where: { $0.id == id }) else { return }
        let task = state.tasks[index]
        counter += 1
        let key = "\(draft.agentKind):mock-new-\(counter)"
        let title = String(draft.text.split(separator: "\n").first ?? "New task").prefix(60)
        let kindLabel = draft.agentLabel
        let ms = MockData.ms(now())
        let project = ProjectRef(key: "proj-" + draft.workspaceId, name: draft.workspaceName)
        let view = AgentView(
            key: key, kind: draft.agentKind, kindLabel: kindLabel, sessionId: "mock-new-\(counter)", title: String(title), project: project,
            state: "working", startedAt: ms, lastActivityAt: ms, turnSteps: 1, currentTool: "Read", canInterject: CanInterject(ok: true),
            followed: true
        )
        let mac = state.macs.first { $0.id == task.macId }
        let item = AgentItem(macId: task.macId, macName: mac?.name ?? "Mac", view: view)
        launchedDrafts[item.id] = draft
        state.agents.insert(item, at: 0)
        state.tasks[index].stage = "watching"
        state.tasks[index].agentKey = key
        publish()
    }

    public func retryTask(id: String) async throws {
        guard let index = state.tasks.firstIndex(where: { $0.id == id }), state.tasks[index].canRetry else { return }
        try requireOnline(state.tasks[index].macId)
        state.tasks[index].stage = "sending"
        state.tasks[index].reason = nil
        publish()
        guard let draft = taskDrafts[id] else { return }
        runTask(id, draft: draft, failure: nil)
    }

    public func dismissTask(id: String) async {
        taskTasks[id]?.cancel()
        taskTasks[id] = nil
        taskDrafts[id] = nil
        state.tasks.removeAll { $0.id == id }
        publish()
    }

    public func takeSlot(macId: String) async throws {
        try requireOnline(macId)
        guard let index = state.macs.firstIndex(where: { $0.id == macId }) else { return }
        await pause(0.3)
        state.macs[index].slot = SlotInfo(active: true, autoActivation: state.macs[index].slot?.autoActivation ?? true)
        publish()
    }

    // MARK: demo controls

    /// A followed Agent finishes, fails or asks for an answer.
    public func simulateFollowEvent(
        macId: String = MockData.homeMacId, agentKey: String = MockData.workingAgentKey, kind: String = "done"
    ) {
        guard let item = state.agents.first(where: { $0.macId == macId && $0.agentKey == agentKey }) else { return }
        let preview = kind == "failed" ? "The run stopped with an error." : "All 14 tests pass; ready for review."
        eventOut.yield(.followEvent(
            macId: macId,
            event: FollowEvent(agentKey: agentKey, kind: kind, title: item.view.title, project: item.view.project.name, preview: preview)
        ))
    }

    public func setFollowLimit(_ limit: Int) {
        followLimit = limit
    }

    public func setFailNextTask(_ reason: String?) {
        failNextTask = reason
    }

    /// Makes the Macs list no models, like an older Mac (the form then has no model or effort row).
    public func setOffersModels(_ offers: Bool) {
        offersModels = offers
    }

    public func setInterjectConsumeDelay(_ seconds: Double?) {
        interjectConsumeDelay = seconds
    }

    public func setNextInterjectFailure(_ reason: String?) {
        failNextInterject = reason
    }

    /// The Agent disappears from the list (its session was cleaned up on the Mac).
    public func dropAgent(macId: String, agentKey: String) {
        state.agents.removeAll { $0.macId == macId && $0.agentKey == agentKey }
        let id = AgentItem.key(macId, agentKey)
        watchTasks[id]?.cancel()
        watchTasks[id] = nil
        state.watches[id] = nil
        publish()
    }

    /// Moves an Agent into another state (for tests of the list and the detail screen).
    public func setAgentResult(macId: String, agentKey: String, result: AgentTurnResult) {
        guard let index = agentIndex(macId, agentKey) else { return }
        state.agents[index].view.state = "idle"
        state.agents[index].view.lastTurnResult = result
        state.agents[index].view.lastTurnFailed = result.outcome == "failed"
        publish()
    }

    public func setAgentState(macId: String, agentKey: String, to newState: String) {
        guard let index = agentIndex(macId, agentKey) else { return }
        state.agents[index].view.state = newState
        state.agents[index].view.canInterject = newState == "working" ? CanInterject(ok: true) : CanInterject(ok: false, reason: newState == "ended" ? "ended" : "notWorking")
        publish()
    }
}
