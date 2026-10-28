import Foundation
import AHProtocol
import AHStore

// Todos of the Macs (docs/specs/ios-todos.md §7): a Mac's projects, the todos of a project, writes to
// them, and the nudge that tells the screen something changed.
//
// Every call is a request with exactly one answer; the answers are routed by key to whoever waits.
// Nothing here is kept: the todos live on the Macs, the phone only shows them.

/// The answer a todo request waits for.
enum TodoReply: Sendable {
    case projects(TodoProjects)
    case list(ProjectTodos)
    case ack(TodoAck)
}

extension PhoneEngine {
    static let todoReadTimeout: TimeInterval = 10
    static let todoWriteTimeout: TimeInterval = 20

    static func todoProjectsKey(_ macId: String) -> String { "p|" + macId }
    static func todoListKey(_ macId: String, _ projectId: String) -> String { "l|" + macId + "|" + projectId }
    static func todoAckKey(_ clientMsgId: String) -> String { "a|" + clientMsgId }

    // MARK: answers

    func onTodoProjects(_ macId: String, _ message: TodoProjects) {
        deliverTodo(.projects(message), key: Self.todoProjectsKey(macId), from: macId)
    }

    func onTodoList(_ macId: String, _ message: ProjectTodos) {
        deliverTodo(.list(message), key: Self.todoListKey(macId, message.id), from: macId)
    }

    func onTodoAck(_ macId: String, _ message: TodoAck) {
        deliverTodo(.ack(message), key: Self.todoAckKey(message.clientMsgId), from: macId)
    }

    /// An answer nobody waits for any more (the request timed out) is dropped.
    private func deliverTodo(_ reply: TodoReply, key: String, from macId: String) {
        guard todoBoxes[key]?.owner == macId else { return }
        todoBoxes[key]?.deliver(reply)
    }

    func onTodoChanged(_ macId: String, _ message: TodoChanged) {
        guard wantedTodoWatches.contains(macId) else { return }
        emit(.todoChanged(macId: macId, rev: message.rev))
    }

    // MARK: requests

    public func todoProjects(macId: String) async throws -> TodoProjects {
        let reply = try await todoRequest(
            TodoProjectsGet(), macId: macId, key: Self.todoProjectsKey(macId), timeout: Self.todoReadTimeout
        )
        guard case .projects(let projects) = reply else { throw BackendError.invalid("reply") }
        return projects
    }

    public func todoList(macId: String, projectId: String) async throws -> ProjectTodos {
        let reply = try await todoRequest(
            TodoListGet(id: projectId), macId: macId, key: Self.todoListKey(macId, projectId), timeout: Self.todoReadTimeout
        )
        guard case .list(let list) = reply else { throw BackendError.invalid("reply") }
        return list
    }

    /// Sends a write and waits for the Mac's verdict. A refusal is an answer, not an error: it comes back as
    /// a `TodoAck` with the project's current list. Only the lack of an answer throws; the write may then
    /// have happened, and sending the same `operation` again (same `clientMsgId`) is safe.
    public func todoApply(macId: String, operation: TodoOperation) async throws -> TodoAck {
        let reply = try await todoRequest(
            operation, macId: macId, key: Self.todoAckKey(operation.clientMsgId), timeout: Self.todoWriteTimeout
        )
        guard case .ack(let ack) = reply else { throw BackendError.invalid("reply") }
        return ack
    }

    /// Asks the Mac to say when its todos change (`todo.changed`), until switched off. The wish survives
    /// reconnects: the Mac forgets subscriptions with the connection, so it is repeated after every handshake.
    public func todoWatch(macId: String, on: Bool) async {
        if on { wantedTodoWatches.insert(macId) } else { wantedTodoWatches.remove(macId) }
        guard macs[macId]?.sync == .synced, macs[macId]?.caps?.todos == true else { return }
        await send(TodoWatch(on: on), to: macId)
    }

    /// After a handshake: subscribe again where the screen still wants it.
    func reopenTodoWatch(_ macId: String) async {
        guard wantedTodoWatches.contains(macId), macs[macId]?.caps?.todos == true else { return }
        await send(TodoWatch(on: true), to: macId)
    }

    private func todoRequest<M: WireMessage>(_ message: M, macId: String, key: String, timeout: TimeInterval) async throws -> TodoReply {
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }
        guard macs[macId]?.caps?.todos == true else { throw BackendError.rejected("unsupported") }
        guard todoBoxes[key] == nil else { throw BackendError.unavailable("busy") }
        todoBoxes[key] = ReplyMailbox(owner: macId)
        defer {
            todoBoxes[key] = nil
            cancelTimer("todo." + key)
        }
        guard await send(message, to: macId) else { throw BackendError.macOffline }
        return try await nextTodoReply(key, timeout: timeout)
    }

    private func nextTodoReply(_ key: String, timeout: TimeInterval) async throws -> TodoReply {
        if let queued = todoBoxes[key]?.popReply() { return queued }
        after("todo." + key, timeout) { engine in await engine.failTodoWait(key, with: BackendError.timedOut) }
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<TodoReply, Error>) in
                if todoBoxes[key] != nil {
                    todoBoxes[key]?.waiter = cont
                } else {
                    cont.resume(throwing: BackendError.cancelled)
                }
            }
        } onCancel: {
            Task { await self.failTodoWait(key, with: BackendError.cancelled) }
        }
    }

    func failTodoWait(_ key: String, with error: BackendError) {
        todoBoxes[key]?.fail(error)
    }

    /// The Mac (or the connection) went away: whoever waits for it is told (all Macs when `macId` is `nil`).
    func failTodos(for macId: String?, with error: BackendError) {
        for (key, box) in todoBoxes where macId == nil || box.owner == macId {
            todoBoxes[key]?.fail(error)
        }
    }
}
