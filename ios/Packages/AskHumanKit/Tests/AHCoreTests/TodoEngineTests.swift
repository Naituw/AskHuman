import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The engine's side of the todo screen: requests and their answers, the verdict of a write, timeouts,
/// a Mac that goes away, and the change subscription.
final class TodoEngineTests: XCTestCase {
    private let todoCaps = Caps(newTask: true, interject: true, watch: true, todos: true)

    private func pairedHarness(todos: Bool = true) async -> Harness {
        let h = Harness()
        h.mac.caps = todos ? todoCaps : Caps(newTask: true, interject: true, watch: true)
        h.mac.todoProjectsReply = TodoProjects(
            rev: 7, mode: "icloud", sync: TodoSyncInfo(),
            projects: [TodoProjectRow(id: "p1", name: "AskHuman", pending: 1, scope: "synced", section: "synced")]
        )
        h.mac.todoLists["p1"] = ProjectTodos(id: "p1", rev: 7, name: "AskHuman", pending: [TodoItem(id: "t1", text: "One")])
        await h.pair()
        return h
    }

    // MARK: reads

    func testTheProjectsAndTheListOfAProjectComeBackFromTheMac() async throws {
        let h = await pairedHarness()
        let projects = try await h.engine.todoProjects(macId: h.mac.macId)
        XCTAssertEqual(projects.mode, "icloud")
        XCTAssertEqual(projects.projects.map(\.id), ["p1"])

        let list = try await h.engine.todoList(macId: h.mac.macId, projectId: "p1")
        XCTAssertEqual(list.pending.map(\.text), ["One"])
        let missing = try await h.engine.todoList(macId: h.mac.macId, projectId: "gone")
        XCTAssertFalse(missing.found, "a project the Mac does not know is an answer, not an error")
    }

    func testAMacFromBeforeTheTodoScreenIsNotAsked() async throws {
        let h = await pairedHarness(todos: false)
        do {
            _ = try await h.engine.todoProjects(macId: h.mac.macId)
            XCTFail("unsupported")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "unsupported")
        }
        XCTAssertEqual(h.mac.count(TodoProjectsGet.type), 0, "nothing goes out to a Mac that cannot answer")
    }

    func testAnUnknownMacIsRefused() async throws {
        let h = await pairedHarness()
        do {
            _ = try await h.engine.todoProjects(macId: "nobody")
            XCTFail("unknown")
        } catch BackendError.macOffline {}
    }

    func testNoAnswerIsATimeout() async throws {
        let h = await pairedHarness()
        h.mac.ignoreTodoReads = true
        let task = Task { try await h.engine.todoProjects(macId: h.mac.macId) }
        await eventually("the Mac to get the request") { h.mac.count(TodoProjectsGet.type) == 1 }
        await h.pass(seconds: Int(PhoneEngine.todoReadTimeout) + 2)
        do {
            _ = try await task.value
            XCTFail("timeout")
        } catch BackendError.timedOut {}
    }

    func testAMacThatGoesAwayFailsTheRequestAtOnce() async throws {
        let h = await pairedHarness()
        h.mac.ignoreTodoReads = true
        let task = Task { try await h.engine.todoList(macId: h.mac.macId, projectId: "p1") }
        await eventually("the Mac to get the request") { h.mac.count(TodoListGet.type) == 1 }
        await h.mac.publishPresence(online: false)
        do {
            _ = try await task.value
            XCTFail("offline")
        } catch BackendError.macOffline {}
        do {
            _ = try await h.engine.todoProjects(macId: h.mac.macId)
            XCTFail("offline")
        } catch BackendError.macOffline {}
    }

    func testTwoAsksForTheSameThingAtOnceAreNotBothServed() async throws {
        let h = await pairedHarness()
        h.mac.ignoreTodoReads = true
        let first = Task { try await h.engine.todoProjects(macId: h.mac.macId) }
        await eventually("the first request to be out") { h.mac.count(TodoProjectsGet.type) == 1 }
        do {
            _ = try await h.engine.todoProjects(macId: h.mac.macId)
            XCTFail("busy")
        } catch BackendError.unavailable(let reason) {
            XCTAssertEqual(reason, "busy")
        }
        first.cancel()
        _ = try? await first.value
    }

    // MARK: writes

    func testAWriteComesBackWithTheMacsListAndTheIdItMade() async throws {
        let h = await pairedHarness()
        let op = TodoOperation(clientMsgId: "m1", id: "p1", op: "add", text: "Two")
        let ack = try await h.engine.todoApply(macId: h.mac.macId, operation: op)
        XCTAssertEqual(ack.status, "ok")
        XCTAssertEqual(ack.todoId, "t-new")
        XCTAssertEqual(ack.list?.pending.map(\.text), ["Two"])
        XCTAssertEqual(h.mac.todoOps.map(\.clientMsgId), ["m1"])
    }

    func testARefusalIsAnAnswerNotAnError() async throws {
        let h = await pairedHarness()
        h.mac.todoAnswer = { op in
            TodoAck(
                clientMsgId: op.clientMsgId, status: "rejected", reason: "tooLong",
                list: ProjectTodos(id: op.id, rev: 9, name: "AskHuman", pending: [TodoItem(id: "t1", text: "One")])
            )
        }
        let ack = try await h.engine.todoApply(macId: h.mac.macId, operation: TodoOperation(clientMsgId: "m1", id: "p1", op: "add", text: "x"))
        XCTAssertEqual(ack.status, "rejected")
        XCTAssertEqual(ack.reason, "tooLong")
        XCTAssertEqual(ack.list?.rev, 9)
    }

    func testAWriteNobodyAnswersTimesOutAndMayBeSentAgainWithTheSameId() async throws {
        let h = await pairedHarness()
        h.mac.todoAnswer = { _ in nil }
        let op = TodoOperation(clientMsgId: "m1", id: "p1", op: "complete", todoId: "t1")
        let task = Task { try await h.engine.todoApply(macId: h.mac.macId, operation: op) }
        await eventually("the Mac to get the write") { h.mac.todoOps.count == 1 }
        await h.pass(seconds: Int(PhoneEngine.todoWriteTimeout) + 2)
        do {
            _ = try await task.value
            XCTFail("timeout")
        } catch BackendError.timedOut {}

        // The same write again (the Mac would replay its first answer); this time the Mac answers.
        h.mac.todoAnswer = nil
        let ack = try await h.engine.todoApply(macId: h.mac.macId, operation: op)
        XCTAssertEqual(ack.status, "ok")
        XCTAssertEqual(h.mac.todoOps.map(\.clientMsgId), ["m1", "m1"])
    }

    // MARK: change subscription

    func testTheNudgeIsHeardOnlyWhileTheScreenWantsIt() async throws {
        let h = await pairedHarness()
        await h.mac.sendTodoChanged(rev: 3)
        try await Task.sleep(nanoseconds: 60_000_000)
        XCTAssertFalse(h.events.contains(.todoChanged(macId: h.mac.macId, rev: 3)), "nobody asked")

        await h.engine.todoWatch(macId: h.mac.macId, on: true)
        await eventually("the subscription to reach the Mac") { h.mac.todoWatches == [true] }
        await eventually("the Mac's answer to being watched") { h.events.contains(.todoChanged(macId: h.mac.macId, rev: 7)) }
        await h.mac.sendTodoChanged(rev: 8)
        await eventually("the nudge") { h.events.contains(.todoChanged(macId: h.mac.macId, rev: 8)) }

        await h.engine.todoWatch(macId: h.mac.macId, on: false)
        await eventually("the unsubscription") { h.mac.todoWatches == [true, false] }
        await h.mac.sendTodoChanged(rev: 9)
        try await Task.sleep(nanoseconds: 60_000_000)
        XCTAssertFalse(h.events.contains(.todoChanged(macId: h.mac.macId, rev: 9)))
    }

    func testTheSubscriptionIsRepeatedAfterTheMacCameBack() async throws {
        let h = await pairedHarness()
        await h.engine.todoWatch(macId: h.mac.macId, on: true)
        await eventually("the subscription") { h.mac.todoWatches == [true] }

        await h.mac.publishPresence(online: false)
        await eventually("offline") { await h.state().macs.first?.link == .macOffline }
        await h.mac.publishPresence(online: true)
        await eventually("subscribed again after the handshake") { h.mac.todoWatches == [true, true] }
    }

    func testNothingIsSubscribedOnAMacThatCannotServeTodos() async throws {
        let h = await pairedHarness(todos: false)
        await h.engine.todoWatch(macId: h.mac.macId, on: true)
        try await Task.sleep(nanoseconds: 60_000_000)
        XCTAssertTrue(h.mac.todoWatches.isEmpty)
    }
}
