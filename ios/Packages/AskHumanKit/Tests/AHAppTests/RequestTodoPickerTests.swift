import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

/// The project's todos in the answer form (docs/specs/special-replies.md D28/D29) against the mock Macs: when they
/// are offered, how they are picked, and what the Mac does with them.
@MainActor
final class RequestTodoPickerTests: XCTestCase {
    private var backend: MockBackend!
    private var app: AppStore!

    private let home = MockData.homeMacId
    private let projectKey = "proj-humaninloop"

    private func make(_ scenario: MockBackend.Scenario = .todoPick) async {
        backend = MockBackend(scenario: scenario, latency: 0)
        let defaults = UserDefaults(suiteName: "ah-todopick-test-" + UUID().uuidString)!
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        await app.start()
        await eventually("the Macs and the inbox") {
            self.app.state.foreground && self.app.state.macs.count == 2 && !self.app.state.inbox.isEmpty
        }
    }

    private func item(_ requestId: String) throws -> InboxItem {
        try XCTUnwrap(app.state.inbox.first { $0.requestId == requestId })
    }

    private func picker(of requestId: String) throws -> RequestTodoPicker {
        try XCTUnwrap(app.todoPicker(for: try item(requestId)), "\(requestId) names a project")
    }

    private func composer(of requestId: String) throws -> AnswerComposer {
        try XCTUnwrap(app.composer(for: try item(requestId)))
    }

    private func loaded(_ requestId: String) async throws -> RequestTodoPicker {
        let picker = try picker(of: requestId)
        await picker.load()
        return picker
    }

    private func queue() async throws -> [String] {
        let id = TodoProjectId.of(key: projectKey)
        return try await backend.todoList(macId: home, projectId: id).pending.map(\.id)
    }

    private let all = (1...7).map { "todo-pick-\($0)" }

    // MARK: what is offered

    func testTheQueueOfTheRequestsProjectIsListedAsTheMacHasIt() async throws {
        await make()
        let picker = try picker(of: "req-migration")
        XCTAssertEqual(picker.phase, .idle)
        XCTAssertFalse(picker.isVisible, "nothing is shown before the queue was read")
        await picker.load()
        XCTAssertEqual(picker.phase, .loaded)
        XCTAssertEqual(picker.todos.map(\.id), all, "the todo the Agent takes first is the first one")
        XCTAssertEqual(picker.count, 7)
        XCTAssertTrue(picker.isVisible)
        XCTAssertEqual(picker.todos[3].att, 1, "the attachment count comes along")
        XCTAssertEqual(picker.projectId, TodoProjectId.of(key: projectKey))
    }

    func testNothingIsOfferedForASelectOnlyRequest() async throws {
        await make()
        let picker = try await loaded("req-select")
        XCTAssertFalse(picker.listsQueue)
        XCTAssertFalse(picker.isVisible)
        XCTAssertEqual(picker.phase, .idle, "the Mac was not even asked")
    }

    func testNothingIsOfferedWhenTheProjectHasNoTodos() async throws {
        await make()
        for id in all { await backend.simulateTodoGone(macId: home, projectKey: projectKey, todoId: id) }
        let ask = try await loaded("req-migration")
        XCTAssertEqual(ask.phase, .loaded)
        XCTAssertFalse(ask.isVisible, "no card for an empty queue")
        let next = try await loaded("req-next")
        XCTAssertTrue(next.listsQueue, "What's next lists what the Mac has, which is nothing")
        XCTAssertFalse(next.isVisible)
    }

    func testAMacFromBeforeTheTodoPickerIsNeverAsked() async throws {
        await make(.demo)
        let picker = try await loaded("req-migration")
        XCTAssertFalse(picker.macTakesPicks)
        XCTAssertFalse(picker.listsQueue, "What's next keeps the todos its question carries")
        XCTAssertFalse(picker.isVisible)
        XCTAssertEqual(picker.phase, .idle)
    }

    func testAMacThatDoesNotKnowTodosAtAllIsNeverAskedEither() async throws {
        await make(.outdated)
        let picker = try await loaded("req-migration")
        XCTAssertFalse(picker.macTakesPicks)
        XCTAssertFalse(picker.isVisible)
    }

    func testARequestWithoutAProjectHasNoPicker() async throws {
        await make()
        var request = try item("req-migration")
        request.summary.project = ProjectRef(key: "  ", name: "")
        XCTAssertNil(app.todoPicker(for: request))
    }

    func testThePickerIsTheSameOneUntilTheRequestIsGone() async throws {
        await make()
        let first = try picker(of: "req-migration")
        let again = try picker(of: "req-migration")
        let other = try picker(of: "req-next")
        XCTAssertTrue(first === again)
        XCTAssertFalse(first === other, "another request, another picker")
    }

    // MARK: picking

    func testPickingKeepsTheOrderOfTheQueueAndTogglingLetsGo() async throws {
        await make()
        let picker = try await loaded("req-migration")
        let composer = try composer(of: "req-migration")
        picker.toggle("todo-pick-4")
        picker.toggle("todo-pick-2")
        XCTAssertEqual(composer.todoIds, ["todo-pick-2", "todo-pick-4"], "the order the Mac merges them in")
        XCTAssertEqual(picker.pickedCount, 2)
        XCTAssertTrue(picker.isPicked("todo-pick-4"))
        picker.toggle("todo-pick-2")
        XCTAssertEqual(composer.todoIds, ["todo-pick-4"])
        XCTAssertTrue(composer.canSend)
        XCTAssertTrue(composer.hasContent, "a todo alone is an answer")
    }

    func testATodoThatIsNotInTheQueueCannotBePicked() async throws {
        await make()
        let picker = try await loaded("req-migration")
        picker.toggle("todo-nope")
        XCTAssertTrue(try composer(of: "req-migration").todoIds.isEmpty)
    }

    func testNothingCanBePickedBeforeTheQueueIsKnown() async throws {
        await make()
        let picker = try picker(of: "req-migration")
        picker.toggle("todo-pick-1")
        XCTAssertTrue(try composer(of: "req-migration").todoIds.isEmpty)
    }

    func testWhatsNextTakesOneTodoAtATimeAndItReplacesAnOption() async throws {
        await make()
        let picker = try await loaded("req-next")
        let composer = try composer(of: "req-next")
        composer.toggle(question: 0, option: 0)
        XCTAssertEqual(composer.answers[0].selected, [0])
        picker.toggle("todo-pick-2")
        XCTAssertEqual(composer.todoIds, ["todo-pick-2"])
        XCTAssertEqual(composer.answers[0].selected, [], "the todo took the place of the option")
        picker.toggle("todo-pick-3")
        XCTAssertEqual(composer.todoIds, ["todo-pick-3"], "the next task is one thing")
        composer.toggle(question: 0, option: 1)
        XCTAssertEqual(composer.todoIds, [], "and an option takes the todo's place")
    }

    func testWhatsNextListsTheFirstFiveAndShowAllOpensTheRest() async throws {
        await make()
        let picker = try await loaded("req-next")
        XCTAssertEqual(picker.listed.map(\.id), Array(all.prefix(5)))
        XCTAssertTrue(picker.canFold)
        picker.toggle("todo-pick-7")
        XCTAssertEqual(picker.listed.map(\.id), Array(all.prefix(5)) + ["todo-pick-7"], "what is picked stays in sight")
        picker.showsAll = true
        XCTAssertEqual(picker.listed.map(\.id), all)
    }

    func testAShortQueueHasNothingToFold() async throws {
        await make()
        await backend.simulateTodoGone(macId: home, projectKey: projectKey, todoId: "todo-pick-7")
        await backend.simulateTodoGone(macId: home, projectKey: projectKey, todoId: "todo-pick-6")
        let picker = try await loaded("req-next")
        XCTAssertEqual(picker.count, 5)
        XCTAssertFalse(picker.canFold)
    }

    func testWhatsNextListsTheQueueEvenWhenItsQuestionCarriedOtherTodos() async throws {
        await make()
        let picker = try await loaded("req-next")
        let composer = try composer(of: "req-next")
        XCTAssertTrue(picker.listsQueue)
        XCTAssertTrue(composer.ask.questions[0].options.allSatisfy(\.todo), "the demo's question carries three")
        XCTAssertGreaterThan(picker.count, composer.ask.questions[0].options.count)
    }

    func testAChoiceOfATodoTheQuestionCarriedIsLetGoOfOnceTheQueueIsListed() async throws {
        await make()
        let composer = try composer(of: "req-next")
        composer.toggle(question: 0, option: 2)
        XCTAssertEqual(composer.answers[0].selected, [2])
        _ = try await loaded("req-next")
        XCTAssertEqual(composer.answers[0].selected, [], "its row is not on screen any more")
    }

    // MARK: sending

    func testSendingTakesThePickedTodosOffTheQueue() async throws {
        await make()
        let picker = try await loaded("req-migration")
        let composer = try composer(of: "req-migration")
        picker.toggle("todo-pick-1")
        picker.toggle("todo-pick-4")
        composer.setText("because", question: 1)
        let request = try item("req-migration")
        try await app.send(composer, item: request)
        await eventually("the request to be answered") { self.app.state.inbox.allSatisfy { $0.requestId != "req-migration" } }
        let sent = await backend.submissions.last
        XCTAssertEqual(sent?.todoIds, ["todo-pick-1", "todo-pick-4"])
        XCTAssertEqual(sent?.answers.last?.text, "because")
        let left = try await queue()
        XCTAssertEqual(left, ["todo-pick-2", "todo-pick-3", "todo-pick-5", "todo-pick-6", "todo-pick-7"])
    }

    func testATodoAloneIsSentAsTheAnswer() async throws {
        await make()
        let picker = try await loaded("req-migration")
        let composer = try composer(of: "req-migration")
        picker.toggle("todo-pick-3")
        try await app.send(composer, item: try item("req-migration"))
        let sent = await backend.submissions.last
        XCTAssertEqual(sent?.todoIds, ["todo-pick-3"])
        XCTAssertEqual(sent?.answers, [], "no item: the ids are the answer")
    }

    func testWhatsNextSendsTheTodoByItsId() async throws {
        await make()
        let picker = try await loaded("req-next")
        let composer = try composer(of: "req-next")
        picker.toggle("todo-pick-6")
        try await app.send(composer, item: try item("req-next"))
        let sent = await backend.submissions.last
        XCTAssertEqual(sent?.todoIds, ["todo-pick-6"], "a todo past the question's own ten is the same as one in it")
        XCTAssertEqual(sent?.answers, [])
    }

    func testAMacFromBeforeTheTodoPickerGetsNoIdsEvenFromADraft() async throws {
        await make(.demo)
        let composer = try composer(of: "req-migration")
        composer.setTodoIds(["todo-pick-1"])
        try await app.send(composer, item: try item("req-migration"))
        let sent = await backend.submissions.last
        XCTAssertEqual(sent?.todoIds, [], "it would drop them, and the answer would go without them silently")
    }

    // MARK: the queue changes

    func testAPickThatIsGoneIsLetGoOfWhenTheQueueIsReadAgain() async throws {
        await make()
        let picker = try await loaded("req-migration")
        let composer = try composer(of: "req-migration")
        picker.toggle("todo-pick-2")
        picker.toggle("todo-pick-3")
        await backend.simulateTodoGone(macId: home, projectKey: projectKey, todoId: "todo-pick-3")
        picker.reload()
        await eventually("the queue to be read again") { picker.count == 6 }
        XCTAssertEqual(composer.todoIds, ["todo-pick-2"])
    }

    func testTheMacRefusesAnAnswerThatNamesATodoThatIsGone() async throws {
        await make()
        let picker = try await loaded("req-migration")
        let composer = try composer(of: "req-migration")
        picker.toggle("todo-pick-5")
        await backend.simulateTodoGone(macId: home, projectKey: projectKey, todoId: "todo-pick-5")
        try await app.send(composer, item: try item("req-migration"))
        await eventually("the refusal") {
            if case .failed(_, let reason)? = self.app.state.inbox.first(where: { $0.requestId == "req-migration" })?.state {
                return reason == Wire.todoGone
            }
            return false
        }
        let left = try await queue()
        XCTAssertEqual(left.count, 6, "nothing was taken")

        // The page reads the queue again (the screen does it on seeing the refusal) and the pick is gone.
        picker.reload()
        await eventually("the pick to be let go of") { composer.todoIds.isEmpty }
        await app.discardFailure(try item("req-migration"))
        await eventually("the request to be answerable again") {
            if case .pending? = self.app.state.inbox.first(where: { $0.requestId == "req-migration" })?.state { return true }
            return false
        }
    }

    func testTheQueueStaysOnScreenWhileTheMacIsAwayAndIsReadAgainWhenItIsBack() async throws {
        await make()
        let picker = try await loaded("req-migration")
        let composer = try composer(of: "req-migration")
        await backend.setLink(.macOffline, for: home)
        await eventually("the Mac to be seen going") { self.app.state.macs.first { $0.id == self.home }?.link == .macOffline }
        XCTAssertTrue(picker.isVisible, "a card that vanishes and comes back is a flicker")
        picker.toggle("todo-pick-2")
        XCTAssertEqual(composer.todoIds, ["todo-pick-2"], "the draft can still be put together")

        await backend.simulateTodoGone(macId: home, projectKey: projectKey, todoId: "todo-pick-7")
        await backend.setLink(.online, for: home)
        await eventually("the queue to be read again when the Mac is back") { picker.count == 6 }
    }

    func testAnUnansweredReadFallsBackToWhatTheQuestionCarries() async throws {
        await make()
        await backend.setUnreachable([home])
        let picker = try await loaded("req-next")
        XCTAssertEqual(picker.phase, .failed)
        XCTAssertFalse(picker.listsQueue, "What's next shows its own todos then")
        XCTAssertFalse(picker.isVisible)
        picker.toggle("todo-pick-1")
        XCTAssertTrue(try composer(of: "req-next").todoIds.isEmpty)

        await backend.setUnreachable([])
        await picker.load()
        XCTAssertEqual(picker.phase, .loaded, "a later look tries again")
        XCTAssertTrue(picker.isVisible)
    }

    func testTheQueueIsReadOnceAtATime() async throws {
        await make()
        let picker = try picker(of: "req-migration")
        async let first: Void = picker.load()
        async let second: Void = picker.load()
        _ = await (first, second)
        XCTAssertEqual(picker.phase, .loaded)
        XCTAssertEqual(picker.count, 7)
    }

    func testAQueueOnScreenIsNotHiddenWhileItIsReadAgain() async throws {
        await make()
        let picker = try await loaded("req-migration")
        let reading = Task { await picker.load() }
        for _ in 0..<20 {
            XCTAssertTrue(picker.isVisible, "the page coming up again does not blank the card")
            await Task.yield()
        }
        await reading.value
        XCTAssertTrue(picker.isVisible)
    }
}
