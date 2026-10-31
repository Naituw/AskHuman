import Foundation
import AHProtocol
import AHStore

// New tasks (plan §4.7): what a Mac offers (workspaces, Agents, TODOs), the creation of a task and
// its progress until the Agent session shows up, plus the "take over questions" button.
//
// A task that failed for a known reason is final: sending the same form again is a new task. Only a
// task whose outcome is unknown (the connection dropped before the Mac answered) is sent again
// under the same id, which the Mac answers from its journal instead of launching twice.

extension PhoneEngine {
    static let taskOptionsTimeout: TimeInterval = 15
    static let taskAcceptTimeout: TimeInterval = 20
    static let maxTrackedTasks = 20

    // MARK: choices

    func onTaskOptions(_ macId: String, _ options: TaskOptions) {
        cancelTimer("taskopts." + macId)
        taskOptions[macId] = .ready(options)
    }

    func onTaskTodos(_ macId: String, _ message: TaskTodos) {
        taskTodos[EngineState.todosKey(macId: macId, workspaceId: message.workspaceId)] = message.todos
    }

    public func loadTaskOptions(macId: String) async {
        guard macs[macId]?.sync == .synced else {
            taskOptions[macId] = .failed("offline")
            publish()
            return
        }
        taskOptions[macId] = .loading
        publish()
        guard await send(TaskOptionsGet(), to: macId) else {
            taskOptions[macId] = .failed("offline")
            publish()
            return
        }
        after("taskopts." + macId, Self.taskOptionsTimeout) { engine in await engine.taskOptionsTimedOut(macId) }
    }

    func taskOptionsTimedOut(_ macId: String) {
        guard case .loading = taskOptions[macId] else { return }
        taskOptions[macId] = .failed("timeout")
        publish()
    }

    public func loadTaskTodos(macId: String, workspaceId: String) async {
        guard macs[macId]?.sync == .synced else { return }
        await send(TaskTodosGet(workspaceId: workspaceId), to: macId)
    }

    // MARK: creating

    public func createTask(macId: String, draft: TaskDraft, progress: @escaping @Sendable (Double) -> Void) async throws -> String {
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }
        guard draft.files.count <= Wire.maxReplyAttachments, draft.files.allSatisfy({ !$0.data.isEmpty && $0.data.count <= Wire.maxAssetBytes }) else { throw BackendError.tooLarge }
        if !draft.files.isEmpty {
            guard case .ready(let options)? = taskOptions[macId], options.attachmentsSupported == true else { throw BackendError.rejected("attachmentsUnsupported") }
        }
        let id = Bytes.newMsgId()
        var uploadIds: [String] = []
        do {
            for (index, source) in draft.files.enumerated() {
                uploadIds.append(try await performUpload(macId: macId, scope: UploadScope(taskId: id), source: source, progress: { fraction in
                    progress((Double(index) + fraction) / Double(draft.files.count))
                }))
            }
            try Task.checkCancellation()
        } catch {
            for uploadId in uploadIds { await send(UploadCancel(uploadId: uploadId), to: macId) }
            throw error
        }
        let body = TaskCreate(
            clientMsgId: id, workspaceId: draft.workspaceId, agentKind: draft.agentKind, permission: draft.permission,
            todoId: draft.todoId, model: draft.model, effort: draft.effort, speed: draft.speed, files: uploadIds.isEmpty ? nil : uploadIds, text: draft.text
        )
        taskBodies[id] = body
        tasks.insert(
            TaskProgress(
                id: id, macId: macId, workspace: draft.workspaceName, agentLabel: draft.agentLabel, text: draft.text,
                stage: "sending", createdAt: clock.now(), modelLabel: draft.modelLabel, effort: draft.effort, speed: draft.speed, attachmentNames: draft.files.map(\.name)
            ),
            at: 0
        )
        trimTasks()
        publish()
        do {
            try await transmitTask(id: id, macId: macId)
        } catch {
            // A publish failure can still have reached the Mac. Once files were uploaded, keep
            // the prepared task in its progress page so retry uses the same creation and file IDs.
            if draft.files.isEmpty { throw error }
        }
        return id
    }

    /// Sends the stored `task.create` again, under the same id (the outcome is unknown).
    public func retryTask(id: String) async throws {
        guard let task = tasks.first(where: { $0.id == id }), taskBodies[id] != nil,
              task.stage == "lost" || task.stage == "failed" && task.reason == "connection"
        else { return }
        guard macs[task.macId]?.sync == .synced else { throw BackendError.macOffline }
        updateTask(id) { $0.stage = "sending"; $0.reason = nil }
        publish()
        try await transmitTask(id: id, macId: task.macId)
    }

    public func dismissTask(id: String) async {
        tasks.removeAll { $0.id == id }
        taskBodies[id] = nil
        cancelTimer("task." + id)
        publish()
    }

    private func transmitTask(id: String, macId: String) async throws {
        guard let body = taskBodies[id] else { return }
        guard await send(body, to: macId) else {
            // An authenticated Mac receipt is stronger evidence than a missing MQTT PUBACK.
            if let task = tasks.first(where: { $0.id == id }), task.stage != "sending" { return }
            updateTask(id) { $0.stage = "failed"; $0.reason = "connection" }
            publish()
            throw BackendError.macOffline
        }
        after("task." + id, Self.taskAcceptTimeout) { engine in await engine.taskNotAccepted(id) }
    }

    func taskNotAccepted(_ id: String) {
        guard tasks.first(where: { $0.id == id })?.stage == "sending" else { return }
        updateTask(id) { $0.stage = "failed"; $0.reason = "connection" }
        publish()
    }

    func onTaskStatus(_ macId: String, _ status: TaskStatus) {
        guard let task = tasks.first(where: { $0.id == status.clientMsgId }), task.macId == macId else { return }
        cancelTimer("task." + status.clientMsgId)
        // A stage never goes back: a late `accepted` must not undo `launched`.
        // A local connection failure is still awaiting the Mac's authoritative outcome.
        if !task.canRetry && (task.isFinal || Self.stageRank(status.stage) < Self.stageRank(task.stage)) { return }
        updateTask(status.clientMsgId) {
            $0.stage = status.stage
            $0.reason = status.reason
            if let key = status.agentKey { $0.agentKey = key }
        }
        if status.stage == "failed" || status.stage == "unmatched" || status.stage == "watching" {
            taskBodies[status.clientMsgId] = nil
        }
    }

    static func stageRank(_ stage: String) -> Int {
        switch stage {
        case "sending", "lost": return 0
        case "accepted": return 1
        case "launching": return 2
        case "launched": return 3
        default: return 4
        }
    }

    func updateTask(_ id: String, _ change: (inout TaskProgress) -> Void) {
        guard let index = tasks.firstIndex(where: { $0.id == id }) else { return }
        change(&tasks[index])
    }

    func trimTasks() {
        while tasks.count > Self.maxTrackedTasks, let last = tasks.last(where: { $0.isFinal }) ?? tasks.last {
            tasks.removeAll { $0.id == last.id }
            taskBodies[last.id] = nil
            cancelTimer("task." + last.id)
        }
    }

    /// The Mac (or the connection) went away: tasks without a final answer are marked as such.
    /// Sending ones can be sent again safely; later ones may well have started.
    func interruptTasks(of macId: String? = nil, forget: Bool = false) {
        for task in tasks where macId == nil || task.macId == macId {
            if forget {
                cancelTimer("task." + task.id)
                taskBodies[task.id] = nil
                continue
            }
            guard !task.isFinal, task.stage != "lost" else { continue }
            cancelTimer("task." + task.id)
            if task.stage == "sending" {
                updateTask(task.id) { $0.stage = "failed"; $0.reason = "connection" }
            } else {
                updateTask(task.id) { $0.stage = "lost"; $0.reason = nil }
                taskBodies[task.id] = nil
            }
        }
        if forget, let macId { tasks.removeAll { $0.macId == macId } }
    }

    // MARK: watches of a Mac that went away

    /// The frames of a Mac are void once it is gone; the wish to watch stays unless `forget`.
    func dropWatches(of macId: String, forget: Bool = false) {
        for id in watches.keys where id.hasPrefix(macId + "|") { watches[id] = nil }
        if forget { wantedWatches = wantedWatches.filter { !$0.hasPrefix(macId + "|") } }
    }

    // MARK: slot

    public func takeSlot(macId: String) async throws {
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }
        guard await send(SlotTake(clientMsgId: Bytes.newMsgId()), to: macId) else { throw BackendError.macOffline }
    }
}
