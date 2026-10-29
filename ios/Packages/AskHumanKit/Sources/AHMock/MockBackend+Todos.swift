import Foundation
import AHProtocol
import AHCore

// Todos of the mock Macs. The Home MacBook Pro syncs through iCloud (`mode: icloud`), the Office iMac
// does not. Writes behave like the real Mac's: they are idempotent by `clientMsgId`, refuse empty and
// over-long text, answer `notFound` for a todo that is gone, and carry the project's list back.

extension MockBackend {
    private func requireTodoMac(_ macId: String) throws {
        guard let mac = state.macs.first(where: { $0.id == macId }) else { throw BackendError.unknownMac }
        if unreachable.contains(macId) || mac.link != .online { throw BackendError.macOffline }
        guard mac.caps.todos else { throw BackendError.rejected("unsupported") }
    }

    private func todoMode(_ macId: String) -> String { syncingTodoMacs.contains(macId) ? "icloud" : "local" }

    private func todoKey(_ macId: String, _ projectId: String) -> String { macId + "|" + projectId }

    /// The list as the Mac would send it now.
    private func todoSnapshot(_ macId: String, _ projectId: String) -> ProjectTodos {
        guard var list = todoLists[todoKey(macId, projectId)] else {
            return ProjectTodos(id: projectId, found: false, rev: todoRev, name: "", canAuto: false)
        }
        list.rev = todoRev
        return list
    }

    // MARK: reads

    public func todoProjects(macId: String) async throws -> TodoProjects {
        try requireTodoMac(macId)
        await pause(0.15)
        try requireTodoMac(macId)
        var rows = todoRows[macId] ?? []
        for index in rows.indices {
            if let list = todoLists[todoKey(macId, rows[index].id)] {
                rows[index].pending = list.pendingTotal
                rows[index].history = list.historyTotal
            }
        }
        let mode = todoMode(macId)
        return TodoProjects(
            rev: todoRev, mode: mode, sync: mode == "icloud" ? MockData.todoSync(now: now(), problem: todoSyncProblem) : nil,
            projects: rows
        )
    }

    public func todoList(macId: String, projectId: String) async throws -> ProjectTodos {
        try requireTodoMac(macId)
        await pause(0.12)
        try requireTodoMac(macId)
        return todoSnapshot(macId, projectId)
    }

    // MARK: writes

    public func todoApply(macId: String, operation: TodoOperation) async throws -> TodoAck {
        try requireTodoMac(macId)
        await pause(0.25)
        try requireTodoMac(macId)
        todoOps.append(operation)
        if let again = todoAcks[operation.clientMsgId] { return again }
        let ack = applyTodo(macId: macId, operation: operation)
        todoAcks[operation.clientMsgId] = ack
        return ack
    }

    private func applyTodo(macId: String, operation: TodoOperation) -> TodoAck {
        let key = todoKey(macId, operation.id)
        func reply(_ status: String, reason: String? = nil, todoId: String? = nil) -> TodoAck {
            let list = todoSnapshot(macId, operation.id)
            return TodoAck(
                clientMsgId: operation.clientMsgId, status: status, reason: reason, todoId: todoId, list: list.found ? list : nil
            )
        }
        if let reason = failNextTodoWrite {
            failNextTodoWrite = nil
            return reply("failed", reason: reason)
        }
        guard var list = todoLists[key] else { return reply("notFound", reason: "unknownProject") }

        let mutation: TodoMutation
        var created: String?
        switch operation.op {
        case "add", "edit":
            guard case .success(let text) = TodoText.validate(operation.text ?? "") else {
                let tooLong = (operation.text ?? "").count > Wire.todoTextMaxChars
                return reply("rejected", reason: tooLong ? "tooLong" : "empty")
            }
            if operation.op == "add" {
                if operation.auto == true, !list.canAuto { return reply("rejected", reason: "cloudOnly") }
                counter += 1
                let id = "todo-mock-\(counter)"
                created = id
                mutation = .add(localId: id, text: text, auto: operation.auto ?? false)
            } else {
                guard let id = operation.todoId, list.pending.contains(where: { $0.id == id }) else {
                    return reply("notFound", reason: "notFound")
                }
                mutation = .edit(id: id, text: text)
            }
        case "complete", "remove", "restore", "auto":
            guard let id = operation.todoId else { return reply("rejected", reason: "invalid") }
            let known = operation.op == "restore" ? list.history.contains { $0.id == id } : list.pending.contains { $0.id == id }
            guard known else { return reply("notFound", reason: "notFound") }
            switch operation.op {
            case "complete": mutation = .complete(id: id)
            case "remove": mutation = .remove(id: id)
            case "restore": mutation = .restore(id: id)
            default:
                if operation.auto == true, !list.canAuto { return reply("rejected", reason: "cloudOnly") }
                mutation = .setAuto(id: id, auto: operation.auto ?? false)
            }
        case "reorder":
            mutation = .reorder(ids: operation.order ?? [])
        default:
            return reply("rejected", reason: "invalid")
        }
        mutation.apply(to: &list, now: now())
        todoLists[key] = list
        bumpTodoRev(macId)
        return reply("ok", todoId: created)
    }

    private func bumpTodoRev(_ macId: String) {
        todoRev += 1
        if todoWatchers.contains(macId) { eventOut.yield(.todoChanged(macId: macId, rev: todoRev)) }
    }

    // MARK: watching

    public func todoWatch(macId: String, on: Bool) async {
        if on {
            todoWatchers.insert(macId)
            // The real Mac says where it stands as soon as it is asked: the phone compares and catches up.
            eventOut.yield(.todoChanged(macId: macId, rev: todoRev))
        } else {
            todoWatchers.remove(macId)
        }
    }

    /// A todo appears from elsewhere (another Mac, an Agent): the phone hears `todo.changed` when it watches.
    public func simulateTodo(macId: String, projectId: String, text: String) {
        let key = todoKey(macId, projectId)
        guard var list = todoLists[key] else { return }
        counter += 1
        TodoMutation.add(localId: "todo-remote-\(counter)", text: text, auto: false).apply(to: &list, now: now())
        todoLists[key] = list
        bumpTodoRev(macId)
    }
}

// MARK: test hooks

extension MockBackend {
    public func setFailNextTodoWrite(_ reason: String?) { failNextTodoWrite = reason }

    public func setTodoSyncProblem(_ problem: String?) { todoSyncProblem = problem }

    /// The Home MacBook Pro is in the middle of a pull: every answer says so until it is called with `false`.
    public func setTodoSyncRunning(_ running: Bool) { todoSyncProblem = running ? MockData.todoSyncRunning : nil }

    /// Which Macs sync their todos through iCloud.
    public func setTodoSyncing(_ macIds: Set<String>) { syncingTodoMacs = macIds }

    public func isWatchingTodos(macId: String) -> Bool { todoWatchers.contains(macId) }

    /// A Mac comes or goes.
    public func setLink(_ link: MacLink, for macId: String) {
        guard let index = state.macs.firstIndex(where: { $0.id == macId }) else { return }
        state.macs[index].link = link
        publish()
    }
}
