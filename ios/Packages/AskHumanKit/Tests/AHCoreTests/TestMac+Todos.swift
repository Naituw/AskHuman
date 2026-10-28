import Foundation
import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The todo side of the fake Mac: the project list, the lists, the writes and the change nudge.
extension TestMac {
    func handleTodos(_ plain: Plain, from phoneId: String) async {
        switch plain.t {
        case TodoProjectsGet.type:
            guard !ignoreTodoReads else { return }
            await sendDown(lock.withLock { todoProjectsReply }, to: phoneId)
        case TodoListGet.type:
            guard !ignoreTodoReads, let get = try? plain.body(as: TodoListGet.self) else { return }
            let list = lock.withLock { todoLists[get.id] } ?? ProjectTodos(id: get.id, found: false, name: "")
            await sendDown(list, to: phoneId)
        case TodoOperation.type:
            guard let op = try? plain.body(as: TodoOperation.self) else { return }
            lock.withLock { todoOps.append(op) }
            let answer = lock.withLock { todoAnswer }
            let ack = answer.map { $0(op) } ?? Self.okAck(op)
            if let ack { await sendDown(ack, to: phoneId) }
        case TodoWatch.type:
            guard let watch = try? plain.body(as: TodoWatch.self) else { return }
            lock.withLock { todoWatches.append(watch.on) }
            if watch.on {
                let rev = lock.withLock { todoProjectsReply.rev }
                await sendDown(TodoChanged(rev: rev), to: phoneId)
            }
        default:
            break
        }
    }

    /// A write that went through: the project holds just the todo that was added (or nothing).
    static func okAck(_ op: TodoOperation) -> TodoAck {
        TodoAck(
            clientMsgId: op.clientMsgId, status: "ok", todoId: op.op == "add" ? "t-new" : nil,
            list: ProjectTodos(id: op.id, rev: 2, name: "P", pending: op.op == "add" ? [TodoItem(id: "t-new", text: op.text ?? "")] : [])
        )
    }

    func sendTodoChanged(rev: UInt64) async {
        for phone in pairedPhones { await sendDown(TodoChanged(rev: rev), to: phone) }
    }
}
