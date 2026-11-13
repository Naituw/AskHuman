import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// Todos over the real gateway (`docs/specs/ios-todos.md`): the throwaway Mac's own `todos.json` is
/// filled and read through its CLI (`AskHuman todo …`, which works on the git root of its working
/// directory), the phone sees and changes it through the engine. Every todo text here is made up.
final class TodoE2ETests: E2ECase {
    // MARK: helpers

    /// A git repository under the throwaway Mac's home: the project the CLI's `todo` commands work on.
    private func repository(_ name: String, parent: String? = nil) throws -> URL {
        var url = mac.home
        if let parent { url = url.appendingPathComponent(parent) }
        url = url.appendingPathComponent(name)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: url.appendingPathComponent(".git"), withIntermediateDirectories: true)
        try "ref: refs/heads/main\n".write(to: url.appendingPathComponent(".git/HEAD"), atomically: true, encoding: .utf8)
        return url
    }

    @discardableResult
    private func todo(_ args: [String], in project: URL) async throws -> CmdResult {
        let result = try await mac.cli(["todo"] + args, cwd: project)
        XCTAssertEqual(result.status, 0, "\(args.joined(separator: " ")): \(result.stderr)")
        return result
    }

    private func macId() async throws -> String { try unwrap(await phone.state().macs.first?.id) }

    private func projects() async throws -> TodoProjects {
        try await phone.engine.todoProjects(macId: try await macId())
    }

    private func row(named name: String) async throws -> TodoProjectRow {
        let all = try await projects()
        return try XCTUnwrap(all.projects.first { $0.name == name }, "no project named \(name) in \(all.projects.map(\.name))")
    }

    private func list(_ id: String) async throws -> ProjectTodos {
        try await phone.engine.todoList(macId: try await macId(), projectId: id)
    }

    @discardableResult
    private func apply(
        _ id: String, _ op: String, todoId: String? = nil, text: String? = nil, auto: Bool? = nil, order: [String]? = nil,
        clientMsgId: String = UUID().uuidString, file: StaticString = #filePath, line: UInt = #line
    ) async throws -> TodoAck {
        let ack = try await phone.engine.todoApply(
            macId: try await macId(),
            operation: TodoOperation(clientMsgId: clientMsgId, id: id, op: op, todoId: todoId, text: text, auto: auto, order: order)
        )
        XCTAssertEqual(ack.status, "ok", "\(op): \(ack.reason ?? "")", file: file, line: line)
        return ack
    }

    // MARK: the project list

    func testThePhoneSeesTheProjectsOfTheMacWithoutAnyPath() async throws {
        try await pair()
        let first = try repository("alpha")
        let second = try repository("alpha", parent: "other")
        try await todo(["add", "write the intro"], in: first)
        try await todo(["add", "--auto", "run the tests"], in: first)
        try await todo(["add", "same name, other folder"], in: second)

        let all = try await projects()
        let alphas = all.projects.filter { $0.name == "alpha" }
        XCTAssertEqual(alphas.count, 2, "two projects of the same name stay apart")
        XCTAssertEqual(Set(alphas.map(\.hint)).count, 2, "the hint (a parent folder) tells them apart")
        XCTAssertEqual(alphas.map(\.pending).sorted(), [1, 2])
        XCTAssertEqual(alphas.first { $0.pending == 2 }?.scope, "local")
        // The project paths never leave the Mac: ids are opaque and a hint is a folder name at most.
        let json = String(decoding: try JSONEncoder().encode(all), as: UTF8.self)
        for path in [mac.home.path, mac.home.resolvingSymlinksInPath().path] {
            XCTAssertFalse(json.contains(path), "no absolute path on the wire")
        }
        XCTAssertTrue(alphas.allSatisfy { $0.id.count == 16 && $0.id.allSatisfy(\.isHexDigit) })
    }

    // MARK: reading and writing a project

    func testThePhoneReadsAndChangesTheTodosOfAProject() async throws {
        try await pair()
        let project = try repository("gamma")
        try await todo(["add", "first"], in: project)
        try await todo(["add", "second"], in: project)
        let id = try await row(named: "gamma").id

        var snapshot = try await list(id)
        XCTAssertEqual(snapshot.pending.map(\.text), ["first", "second"])
        XCTAssertEqual(snapshot.pendingTotal, 2)
        XCTAssertTrue(snapshot.history.isEmpty)

        // add: lands at the end, and the Mac's own list agrees.
        let added = try await apply(id, "add", text: "third from the phone")
        let addedId = try unwrap(added.todoId)
        XCTAssertEqual(added.list?.pending.map(\.text), ["first", "second", "third from the phone"])
        let onMac = try await todo(["list"], in: project)
        XCTAssertTrue(onMac.stdout.contains("third from the phone"), onMac.stdout)

        // edit
        try await apply(id, "edit", todoId: addedId, text: "third, edited")
        snapshot = try await list(id)
        XCTAssertEqual(snapshot.pending.map(\.text), ["first", "second", "third, edited"])

        // auto
        try await apply(id, "auto", todoId: addedId, auto: true)
        snapshot = try await list(id)
        XCTAssertEqual(snapshot.pending.last?.auto, true)

        // reorder: the last one first.
        let ids = snapshot.pending.map(\.id)
        try await apply(id, "reorder", order: [ids[2], ids[0], ids[1]])
        snapshot = try await list(id)
        XCTAssertEqual(snapshot.pending.map(\.text), ["third, edited", "first", "second"])

        // complete: leaves the queue, shows up in the history; restore brings it back.
        try await apply(id, "complete", todoId: ids[0])
        snapshot = try await list(id)
        XCTAssertEqual(snapshot.pending.map(\.text), ["third, edited", "second"])
        XCTAssertEqual(snapshot.history.map(\.text), ["first"])
        try await apply(id, "restore", todoId: snapshot.history[0].id)
        snapshot = try await list(id)
        XCTAssertEqual(snapshot.pendingTotal, 3)
        XCTAssertTrue(snapshot.history.isEmpty)

        // remove
        try await apply(id, "remove", todoId: addedId)
        snapshot = try await list(id)
        XCTAssertEqual(snapshot.pendingTotal, 2)
    }

    func testAnAnsweredWriteIsNotAppliedTwiceAndRefusalsAreAnswered() async throws {
        try await pair()
        let project = try repository("delta")
        try await todo(["add", "only one"], in: project)
        let id = try await row(named: "delta").id

        // The same clientMsgId again (a retry after a lost answer) changes nothing the second time.
        let msg = "e2e-" + UUID().uuidString
        try await apply(id, "add", text: "added once", clientMsgId: msg)
        try await apply(id, "add", text: "added once", clientMsgId: msg)
        let afterRetry = try await list(id).pendingTotal
        XCTAssertEqual(afterRetry, 2)

        // Refusals come back as answers, with the list as it is.
        let missing = try await phone.engine.todoApply(
            macId: try await macId(),
            operation: TodoOperation(clientMsgId: UUID().uuidString, id: "0000000000000000", op: "add", text: "nowhere")
        )
        XCTAssertEqual(missing.status, "notFound")
        let empty = try await phone.engine.todoApply(
            macId: try await macId(),
            operation: TodoOperation(clientMsgId: UUID().uuidString, id: id, op: "add", text: "   ")
        )
        XCTAssertEqual(empty.status, "rejected")
        let gone = try await phone.engine.todoApply(
            macId: try await macId(),
            operation: TodoOperation(clientMsgId: UUID().uuidString, id: id, op: "complete", todoId: "not-a-todo")
        )
        XCTAssertEqual(gone.status, "notFound")
        XCTAssertEqual(gone.list?.pendingTotal, 2, "a refusal still brings the phone up to date")
    }

    // MARK: changing the list of a request

    /// The list of todos on the request page (docs/specs/todo-list-actions.md): the same picker, over the real gateway,
    /// with its queue read.
    @MainActor
    private static func requestList(engine: PhoneEngine, item: InboxItem, mac: MacState?) async throws -> RequestTodoPicker {
        let form = AnswerComposer(key: item.id, ask: try XCTUnwrap(item.view?.ask), drafts: MemoryDraftStore())
        let picker = RequestTodoPicker(
            backend: engine, composer: form, macId: item.macId, projectKey: item.summary.project.key, mac: mac
        )
        await picker.load()
        return picker
    }

    /// What the page's add row does with a text, until the Mac has answered the write: the todo is the Mac's by then.
    @MainActor
    private static func addFromTheRequestPage(
        picker: RequestTodoPicker, text: String
    ) async -> RequestTodoPicker.AddResult {
        let result = picker.add(text)
        for _ in 0..<200 where picker.todos.contains(where: { picker.isSending($0.id) }) {
            try? await Task.sleep(nanoseconds: 50_000_000)
        }
        return result
    }

    /// A request that is still waiting names its project, and the Mac takes a todo for it even though that
    /// project is in none of its lists: no todo yet, no hook that registered the folder, no Agent working in
    /// it (docs/specs/todo-quick-add.md D14). The id the phone works out from the request's project key is the
    /// one the Mac uses for the project in its own list.
    func testAWaitingRequestsProjectTakesATodoFromThePhone() async throws {
        try await pair()
        let project = try repository("zeta")
        let before = try await projects()
        XCTAssertNil(before.projects.first { $0.name == "zeta" }, "nothing has told the Mac about this project")

        let ask = try mac.ask(["-q", "Which way?", "-o", "Left", "-o", "Right"], cwd: project)
        let item = try await phone.request()
        XCTAssertTrue(item.hasProject)
        XCTAssertEqual(item.summary.project.name, "zeta")
        let macState = await phone.state().macs.first

        let engine = phone.engine
        let picker = try await Self.requestList(engine: engine, item: item, mac: macState)
        let loaded = await picker.phase
        XCTAssertEqual(loaded, .loaded, "the Mac answers for the project of a request that is still waiting")
        let first = await Self.addFromTheRequestPage(picker: picker, text: "Cover the rollback path")
        XCTAssertEqual(first, .added)
        let second = await Self.addFromTheRequestPage(picker: picker, text: "Ask about the retention")
        XCTAssertEqual(second, .added)
        let shown = await picker.todos.map(\.text)
        XCTAssertEqual(shown, ["Cover the rollback path", "Ask about the retention"], "the list is what the Mac answered")
        let notice = await picker.notice
        XCTAssertNil(notice, "the Mac took both writes")

        // The Mac's own list agrees, in order, and the project is in its project list now under the same id.
        let onMac = try await todo(["list"], in: project)
        XCTAssertTrue(onMac.stdout.contains("Cover the rollback path"), onMac.stdout)
        XCTAssertTrue(onMac.stdout.contains("Ask about the retention"), onMac.stdout)
        let listed = try await row(named: "zeta")
        XCTAssertEqual(listed.id, TodoProjectId.of(key: item.summary.project.key))
        XCTAssertEqual(listed.pending, 2)
        let snapshot = try await list(listed.id)
        XCTAssertEqual(snapshot.pending.map(\.text), ["Cover the rollback path", "Ask about the retention"])
        XCTAssertTrue(snapshot.pending.allSatisfy { !$0.auto }, "a todo from the request page carries no ⚡")

        // The request itself is untouched by all this and can still be answered.
        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [1])]
        )
        await assertFinishes(ask)
        XCTAssertTrue(ask.stdout.contains("Right"), ask.stdout)
    }

    // MARK: picking todos while answering

    /// The project's pending queue; a project with nothing pending (and no request waiting on it) is not listed at all.
    private func pending(_ project: URL) async throws -> [TodoItem] {
        let all = try await projects()
        guard let row = all.projects.first(where: { $0.name == project.lastPathComponent }) else { return [] }
        return try await list(row.id).pending
    }

    /// What the phone's answer form does with the todos it picks (docs/specs/special-replies.md D28): their ids ride
    /// with `answer.submit`, and the Mac puts their text into the answer, takes them off the queue and delivers them.
    func testTodosPickedWhileAnsweringJoinTheAnswerAndLeaveTheQueue() async throws {
        try await pair()
        let project = try repository("theta")
        for text in ["Cover the rollback path", "Update the changelog", "Rename the helper"] {
            try await todo(["add", text], in: project)
        }

        let ask = try mac.ask(["-q", "Which way?", "-o", "Left", "-o", "Right"], cwd: project)
        let item = try await phone.request()
        let queue = try await list(TodoProjectId.of(key: item.summary.project.key)).pending
        XCTAssertEqual(queue.map(\.text), ["Cover the rollback path", "Update the changelog", "Rename the helper"])
        let caps = await phone.state().macs.first?.caps
        XCTAssertEqual(caps?.answerTodos, true, "the Mac announces that it takes picked todos")

        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send",
            answers: [AnswerItem(q: 0, sel: [1], text: "because")], todoIds: [queue[0].id, queue[2].id]
        )
        await assertFinishes(ask)
        XCTAssertTrue(ask.stdout.contains("Right"), ask.stdout)
        XCTAssertTrue(ask.stdout.contains("because"), ask.stdout)
        let first = try XCTUnwrap(ask.stdout.range(of: "Cover the rollback path"), ask.stdout)
        let second = try XCTUnwrap(ask.stdout.range(of: "Rename the helper"), ask.stdout)
        XCTAssertLessThan(first.lowerBound, second.lowerBound, "in the queue's order, as the popup merges them")
        XCTAssertFalse(ask.stdout.contains("Update the changelog"), "what was not picked is not in the answer")

        await eventually("the picked todos to leave the queue") {
            ((try? await self.pending(project))?.map(\.text)) == ["Update the changelog"]
        }
        let id = try await row(named: "theta").id
        let history = try await list(id).history.map(\.text)
        XCTAssertEqual(Set(history), ["Cover the rollback path", "Rename the helper"])
    }

    /// A todo alone is an answer, and What's next takes it by id even when the question did not carry it: the Mac lists
    /// ten of the queue in the question, the phone shows all of it.
    func testWhatsNextTakesATodoBeyondItsTenByIdAndItIsTheNextTask() async throws {
        try await pair()
        let project = try repository("iota")
        for number in 1...12 { try await todo(["add", "Task number \(number)"], in: project) }

        let next = try mac.ask(["--whats-next"], cwd: project)
        let item = try await phone.request()
        XCTAssertEqual(item.view?.ask?.whatsNext, true)
        let carried = item.view?.ask?.questions.first?.options.filter(\.todo).count ?? 0
        XCTAssertLessThanOrEqual(carried, 10, "the question lists at most ten of the queue")
        let queue = try await list(TodoProjectId.of(key: item.summary.project.key)).pending
        XCTAssertEqual(queue.count, 12, "the phone reads the whole queue")

        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [], todoIds: [queue[11].id]
        )
        await assertFinishes(next)
        XCTAssertTrue(next.stdout.contains("Task number 12"), next.stdout)
        await eventually("the picked todo to leave the queue") {
            ((try? await self.pending(project))?.count) == 11
        }
    }

    /// A pick that is gone by the time the Mac looks is refused as a whole; the request stays, and the same answer
    /// without it goes through.
    func testAPickedTodoThatIsGoneRefusesTheAnswerAndTheRequestStays() async throws {
        try await pair()
        let project = try repository("kappa")
        try await todo(["add", "Will be gone"], in: project)
        try await todo(["add", "Stays"], in: project)

        let ask = try mac.ask(["-q", "Ready?", "-o", "Yes", "-o", "No"], cwd: project)
        let item = try await phone.request()
        let id = TodoProjectId.of(key: item.summary.project.key)
        let queue = try await list(id).pending
        _ = try await apply(id, "remove", todoId: queue[0].id)

        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0])],
            todoIds: [queue[0].id, queue[1].id]
        )
        await eventually("the refusal") {
            if case .failed(_, let reason)? = await self.phone.state().inbox.first?.state { return reason == Wire.todoGone }
            return false
        }
        XCTAssertTrue(ask.isRunning, "the request is still waiting")
        let left = try await pending(project).map(\.text)
        XCTAssertEqual(left, ["Stays"], "nothing was taken")

        if case .failed(let clientMsgId, _)? = await phone.state().inbox.first?.state {
            await phone.engine.discardSubmission(clientMsgId: clientMsgId)
        }
        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0])],
            todoIds: [queue[1].id]
        )
        await assertFinishes(ask)
        XCTAssertTrue(ask.stdout.contains("Stays"), ask.stdout)
        let remaining = try await pending(project).map(\.text)
        XCTAssertEqual(remaining, [])
    }

    /// A select-only question takes no todos: the Mac refuses an answer that names one.
    func testASelectOnlyQuestionRefusesPickedTodos() async throws {
        try await pair()
        let project = try repository("lambda")
        try await todo(["add", "Not for this one"], in: project)

        let ask = try mac.ask(["-q", "Pick one", "-o", "A", "-o", "B", "--select-only"], cwd: project)
        let item = try await phone.request()
        let queue = try await pending(project)
        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0])],
            todoIds: [queue[0].id]
        )
        await eventually("the refusal") {
            if case .failed? = await self.phone.state().inbox.first?.state { return true }
            return false
        }
        XCTAssertTrue(ask.isRunning)
        if case .failed(let clientMsgId, _)? = await phone.state().inbox.first?.state {
            await phone.engine.discardSubmission(clientMsgId: clientMsgId)
        }
        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [1])]
        )
        await assertFinishes(ask)
        let remaining = try await pending(project).map(\.text)
        XCTAssertEqual(remaining, ["Not for this one"])
    }

    // MARK: watching

    func testAWatchedListTellsThePhoneWhenTheMacChanges() async throws {
        try await pair()
        let project = try repository("epsilon")
        try await todo(["add", "before the watch"], in: project)
        let macId = try await macId()

        let before = phone.events.count
        await phone.engine.todoWatch(macId: macId, on: true)
        // Turning the watch on answers with where things stand.
        await eventually("the first change notice") {
            self.phone.events.dropFirst(before).contains { if case .todoChanged = $0 { return true } else { return false } }
        }
        let baseline = phone.events.dropFirst(before).compactMap { event -> UInt64? in
            if case .todoChanged(_, let rev) = event { return rev }
            return nil
        }.last ?? 0

        try await todo(["add", "while watching"], in: project)
        await eventually("a change notice with a newer revision") {
            self.phone.events.dropFirst(before).contains { event in
                if case .todoChanged(_, let rev) = event { return rev != baseline }
                return false
            }
        }

        // Off again: further changes are silent.
        await phone.engine.todoWatch(macId: macId, on: false)
        try await Task.sleep(nanoseconds: 1_500_000_000)
        let quiet = phone.events.count
        try await todo(["add", "after the watch"], in: project)
        try await Task.sleep(nanoseconds: 3_000_000_000)
        let late = phone.events.dropFirst(quiet).contains { if case .todoChanged = $0 { return true } else { return false } }
        XCTAssertFalse(late, "a switched-off watch stays quiet")
    }

    // MARK: the daemon's view of the phone

    func testThePhoneIsToldThatTheMacHandlesTodos() async throws {
        try await pair()
        let state = await phone.state()
        XCTAssertTrue(state.macs.first?.caps.todos == true, "the Mac announces todos in its capabilities")
    }
}
