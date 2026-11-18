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

    /// Waits for something to be true of the Mac (which is asked, so the check is async).
    private func waitForMac(
        _ message: String, file: StaticString = #filePath, line: UInt = #line, _ check: @escaping () async throws -> Bool
    ) async throws {
        let deadline = Date().addingTimeInterval(3)
        while Date() < deadline {
            if try await check() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("timed out waiting for: \(message)", file: file, line: line)
    }

    private func history() async throws -> [String] {
        let id = TodoProjectId.of(key: projectKey)
        return try await backend.todoList(macId: home, projectId: id).history.map(\.id)
    }

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

    func testAProjectWithoutTodosOffersTheLineThatAddsTheFirst() async throws {
        await make()
        for id in all { await backend.simulateTodoGone(macId: home, projectKey: projectKey, todoId: id) }
        let ask = try await loaded("req-migration")
        XCTAssertEqual(ask.phase, .loaded)
        XCTAssertTrue(ask.isVisible, "the list is there, to add the first todo to")
        XCTAssertTrue(ask.showsEntry, "a single line, not a card of nothing")
        XCTAssertEqual(ask.count, 0)
        let next = try await loaded("req-next")
        XCTAssertTrue(next.listsQueue, "What's next lists what the Mac has, which is nothing")
        XCTAssertTrue(next.isVisible, "and the row that adds one")
        XCTAssertTrue(next.rows.isEmpty)
    }

    func testAProjectTheMacDoesNotKnowYetIsAnEmptyQueueThatTakesWrites() async throws {
        await make()
        let picker = try await loaded("req-fresh")
        XCTAssertEqual(picker.phase, .loaded, "an empty answer is an answer")
        XCTAssertTrue(picker.showsEntry)
        XCTAssertTrue(picker.canWrite)
        XCTAssertEqual(picker.add("Scaffold the project"), .added)
        XCTAssertFalse(picker.showsEntry, "the line gives way to the list")
        let id = TodoProjectId.of(key: "proj-fresh")
        try await waitForMac("the Mac to hold the todo") {
            try await self.backend.todoList(macId: self.home, projectId: id).pending.map(\.text) == ["Scaffold the project"]
        }
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

    func testMultiChoiceWhatsNextPicksSeveralTodosInQueueOrder() async throws {
        await make()
        var ask = try composer(of: "req-next").ask
        ask.single = false
        let form = AnswerComposer(key: "multi-next", ask: ask, drafts: MemoryDraftStore())
        let picker = RequestTodoPicker(backend: backend, composer: form, macId: home, projectKey: projectKey,
                                      mac: app.state.macs.first { $0.id == home })
        await picker.load()
        XCTAssertFalse(picker.singleSelection)
        picker.toggle("todo-pick-4")
        picker.toggle("todo-pick-2")
        XCTAssertEqual(form.todoIds, ["todo-pick-2", "todo-pick-4"])
        picker.toggle("todo-pick-4")
        XCTAssertEqual(form.todoIds, ["todo-pick-2"])
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

    // MARK: the Stop confirmation

    func testTheStopConfirmationListsTheQueueAndTakesOneTodoAtATime() async throws {
        await make()
        let picker = try await loaded("req-stop-ask")
        let composer = try composer(of: "req-stop-ask")
        XCTAssertTrue(composer.ask.todoQueue)
        XCTAssertTrue(picker.listsQueue, "a queue of its own, not the one todo the question carried")
        XCTAssertTrue(picker.singleSelection)
        XCTAssertEqual(picker.todos.map(\.id), all)
        picker.toggle("todo-pick-2")
        picker.toggle("todo-pick-3")
        XCTAssertEqual(composer.todoIds, ["todo-pick-3"], "what comes next is one thing")
        composer.toggle(question: 0, option: 2)
        XCTAssertEqual(composer.todoIds, [], "ending the conversation takes the todo's place")
    }

    // MARK: completing, deleting, adding

    func testCompletingATodoTakesItOffTheQueueAtOnceAndTheMacPutsItInTheHistory() async throws {
        await make()
        let picker = try await loaded("req-migration")
        picker.complete("todo-pick-3")
        XCTAssertEqual(picker.todos.map(\.id), all.filter { $0 != "todo-pick-3" }, "gone before the Mac has said anything")
        XCTAssertEqual(picker.undo?.todoId, "todo-pick-3")
        XCTAssertEqual(picker.undo?.text, "整理 Markdown 渲染的快照测试")
        try await waitForMac("the Mac to have completed it") { try await self.queue() == self.all.filter { $0 != "todo-pick-3" } }
        let done = try await history()
        XCTAssertEqual(done.first, "todo-pick-3", "completed (a history entry), not deleted")
        XCTAssertNil(picker.notice)
    }

    func testCompletingAPickedTodoLetsGoOfThePick() async throws {
        await make()
        let picker = try await loaded("req-migration")
        let composer = try composer(of: "req-migration")
        picker.toggle("todo-pick-2")
        picker.toggle("todo-pick-4")
        picker.complete("todo-pick-2")
        XCTAssertEqual(composer.todoIds, ["todo-pick-4"], "a todo that is done is not part of the answer")
        picker.remove("todo-pick-4")
        XCTAssertTrue(composer.todoIds.isEmpty)
        XCTAssertFalse(composer.hasContent)
    }

    func testTakingACompletionBackPutsTheTodoWhereItWas() async throws {
        await make()
        let picker = try await loaded("req-migration")
        picker.complete("todo-pick-3")
        picker.undoComplete()
        XCTAssertNil(picker.undo, "one undo per completion")
        XCTAssertEqual(picker.todos.map(\.id), all, "back at once, in the place it had")
        try await waitForMac("complete, restore and reorder") { await self.backend.todoOps.map(\.op) == ["complete", "restore", "reorder"] }
        let back = try await queue()
        XCTAssertEqual(back, all, "the Mac has it where it was too")
        await eventually("the screen to settle") { !picker.todos.contains { picker.isSending($0.id) } }
        XCTAssertEqual(picker.todos.map(\.id), all)
        let done = try await history()
        XCTAssertFalse(done.contains("todo-pick-3"))
    }

    func testTheNoteOfACompletionIsReplacedByTheNextOne() async throws {
        await make()
        let picker = try await loaded("req-migration")
        picker.complete("todo-pick-1")
        let first = try XCTUnwrap(picker.undo)
        picker.complete("todo-pick-2")
        let second = try XCTUnwrap(picker.undo)
        XCTAssertNotEqual(first.id, second.id, "its timer is its own")
        picker.dismissUndo(first.id)
        XCTAssertEqual(picker.undo?.todoId, "todo-pick-2", "the old note's timer does not dismiss the new one")
        picker.dismissUndo(second.id)
        XCTAssertNil(picker.undo)
    }

    func testDeletingATodoRemovesItForGoodWithNoNoteToTakeItBack() async throws {
        await make()
        let picker = try await loaded("req-migration")
        picker.remove("todo-pick-5")
        XCTAssertEqual(picker.todos.map(\.id), all.filter { $0 != "todo-pick-5" })
        XCTAssertNil(picker.undo)
        try await waitForMac("the Mac to have removed it") { try await self.queue() == self.all.filter { $0 != "todo-pick-5" } }
        let done = try await history()
        XCTAssertFalse(done.contains("todo-pick-5"), "removed, not completed")
    }

    func testAddingATodoShowsItAtTheEndAtOnceAndTheMacGivesItItsOwnId() async throws {
        await make()
        let picker = try await loaded("req-migration")
        let composer = try composer(of: "req-migration")
        XCTAssertEqual(picker.add("  Write the release notes \n"), .added)
        let added = try XCTUnwrap(picker.todos.last)
        XCTAssertEqual(added.text, "Write the release notes", "trimmed, and last in the queue")
        XCTAssertEqual(picker.count, 8)
        XCTAssertTrue(picker.isSending(added.id), "the Mac has not given it an id yet")
        XCTAssertFalse(picker.canAct(on: added.id), "so nothing can be done to it yet")
        XCTAssertTrue(picker.isExpanded, "the list opens to show it")
        XCTAssertTrue(picker.isLit(added.id))
        XCTAssertNotNil(picker.revealKey)
        let key = picker.rowKey(added.id)

        await eventually("the Mac to have answered") { !picker.todos.contains { picker.isSending($0.id) } }
        let given = try XCTUnwrap(picker.todos.last)
        XCTAssertNotEqual(given.id, added.id, "the Mac's id replaced the local one")
        XCTAssertEqual(given.text, "Write the release notes")
        XCTAssertEqual(picker.rowKey(given.id), key, "it is the same row on screen: nothing blinks")
        XCTAssertTrue(picker.isLit(given.id), "and it is still lit")
        XCTAssertFalse(given.auto, "a todo added here never runs by itself")
        XCTAssertEqual(given.att, 0)
        XCTAssertFalse(composer.todoIds.contains(given.id), "adding is not answering")
        XCTAssertTrue(picker.canAct(on: given.id))
        let ops = await backend.todoOps
        XCTAssertEqual(ops.last?.op, "add")
        XCTAssertNil(ops.last?.auto)
        XCTAssertNil(picker.notice)
    }

    func testAddingToACollapsedListOpensItToShowTheNewRow() async throws {
        await make()
        let picker = try await loaded("req-next")
        XCTAssertFalse(picker.showsAll)
        XCTAssertEqual(picker.add("The eighth"), .added)
        XCTAssertTrue(picker.showsAll, "five of them are shown at first; the new one is the eighth")
        XCTAssertEqual(picker.rows.last?.todo.text, "The eighth")
    }

    func testATextThatIsNotATodoIsNotAdded() async throws {
        await make()
        let picker = try await loaded("req-migration")
        XCTAssertEqual(picker.add(" \n "), .invalid(.empty))
        XCTAssertEqual(picker.add(String(repeating: "x", count: Wire.todoTextMaxChars + 1)), .invalid(.tooLong))
        XCTAssertEqual(picker.count, 7)
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty, "nothing was sent")
    }

    func testTheAddRowOpensKeepsItsDraftAndGoesBackWhenEmpty() async throws {
        await make()
        let picker = try await loaded("req-migration")
        XCTAssertFalse(picker.isAdding)
        picker.beginAdd()
        XCTAssertTrue(picker.isAdding)
        XCTAssertTrue(picker.addFocused, "the keyboard comes with the input")
        XCTAssertTrue(picker.isExpanded, "even from a folded card")

        picker.addText = "one"
        picker.submitAdd()
        XCTAssertEqual(picker.addText, "", "cleared for the next one")
        XCTAssertTrue(picker.isAdding, "and it stays an input")
        picker.addFocusChanged(false)
        XCTAssertFalse(picker.isAdding, "nothing written and no keyboard: back to the line")

        picker.beginAdd()
        picker.addText = "a draft"
        picker.addFocusChanged(false)
        XCTAssertTrue(picker.isAdding, "a draft is never hidden")
        XCTAssertEqual(picker.addText, "a draft")
    }

    func testATooLongTextStaysInTheInputWithTheReason() async throws {
        await make()
        let picker = try await loaded("req-migration")
        picker.beginAdd()
        picker.addText = String(repeating: "x", count: Wire.todoTextMaxChars + 1)
        picker.submitAdd()
        XCTAssertEqual(picker.addProblem, .tooLong)
        XCTAssertFalse(picker.addText.isEmpty, "what was written is not lost")
        picker.addTextChanged()
        XCTAssertNil(picker.addProblem, "the reason is for the text it was about")
        picker.addText = " "
        picker.submitAdd()
        XCTAssertNil(picker.addProblem, "nothing written is not worth a word")
    }

    func testWritesGoOutInTheOrderTheyWereMade() async throws {
        await make()
        let picker = try await loaded("req-migration")
        picker.complete("todo-pick-1")
        picker.add("Last one")
        picker.remove("todo-pick-7")
        try await waitForMac("all three writes") { await self.backend.todoOps.map(\.op) == ["complete", "add", "remove"] }
        let queued = try await queue()
        XCTAssertEqual(queued.count, 6)
        let done = try await history()
        XCTAssertEqual(done.first, "todo-pick-1")
        let texts = try await backend.todoList(macId: home, projectId: TodoProjectId.of(key: projectKey)).pending.map(\.text)
        XCTAssertEqual(texts.last, "Last one")
        await eventually("the screen to catch up") { !picker.todos.contains { picker.isSending($0.id) } }
        XCTAssertEqual(picker.count, 6)
        XCTAssertEqual(picker.todos.last?.text, "Last one")
    }

    func testALostAnswerIsAskedForAgainAndTheWriteIsNotDoneTwice() async throws {
        await make()
        let picker = try await loaded("req-migration")
        await backend.setLoseNextTodoAnswers(1)
        picker.complete("todo-pick-2")
        try await waitForMac("both attempts") { await self.backend.todoOps.count == 2 }
        try await Task.sleep(for: .milliseconds(50))
        let left = try await queue()
        XCTAssertEqual(left, all.filter { $0 != "todo-pick-2" })
        XCTAssertEqual(picker.count, 6)
        XCTAssertNil(picker.notice, "the same write twice does the same thing once, and so says nothing")
        XCTAssertEqual(picker.undo?.todoId, "todo-pick-2", "a completion that went through can be taken back")
        let ops = await backend.todoOps
        XCTAssertEqual(ops.count, 2)
        XCTAssertEqual(ops[0].clientMsgId, ops[1].clientMsgId)
    }

    func testATodoThatWasDoneElsewhereIsSaidSoAndTheListCatchesUp() async throws {
        await make()
        let picker = try await loaded("req-migration")
        await backend.simulateTodoGone(macId: home, projectKey: projectKey, todoId: "todo-pick-2")
        picker.complete("todo-pick-2")
        await eventually("the Mac's answer") { picker.notice != nil }
        XCTAssertEqual(picker.notice?.notice, .changedElsewhere)
        XCTAssertNil(picker.undo, "a completion that did not take cannot be taken back")
        XCTAssertEqual(picker.todos.map(\.id), all.filter { $0 != "todo-pick-2" }, "the list is the Mac's again")
    }

    func testAWriteTheMacCouldNotStoreDisappearsAndSaysSo() async throws {
        await make()
        let picker = try await loaded("req-migration")
        await backend.setFailNextTodoWrite("diskFull")
        picker.add("Will not stick")
        XCTAssertEqual(picker.count, 8, "shown at once")
        await eventually("the Mac's answer") { picker.notice != nil }
        XCTAssertEqual(picker.notice?.notice, .notSaved)
        XCTAssertEqual(picker.count, 7, "and gone when the Mac did not keep it")
        XCTAssertFalse(picker.todos.contains { $0.text == "Will not stick" })
    }

    func testNothingIsWrittenWhileTheMacIsAwayAndTheListSaysWhy() async throws {
        await make()
        let picker = try await loaded("req-migration")
        await backend.setLink(.macOffline, for: home)
        await eventually("the Mac to be seen going") { self.app.state.macs.first { $0.id == self.home }?.link == .macOffline }
        XCTAssertTrue(picker.isVisible, "the list stays")
        XCTAssertFalse(picker.canWrite)
        XCTAssertTrue(picker.isOffline)
        XCTAssertFalse(picker.canAct(on: "todo-pick-1"))

        picker.complete("todo-pick-1")
        XCTAssertEqual(picker.notice?.notice, .offline)
        XCTAssertEqual(picker.add("Not now"), .unavailable)
        picker.remove("todo-pick-2")
        XCTAssertEqual(picker.count, 7, "nothing changed")
        XCTAssertNil(picker.undo)
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty, "and nothing is kept to be sent later")

        await backend.setLink(.online, for: home)
        await eventually("the Mac to be back") { picker.canWrite }
        XCTAssertFalse(picker.isOffline)
    }

    func testAMacThatLostTheLinkInTheMiddleOfAWriteTakesItBack() async throws {
        await make()
        let picker = try await loaded("req-migration")
        await backend.setUnreachable([home])
        picker.complete("todo-pick-1")
        XCTAssertEqual(picker.count, 6, "shown at once")
        await eventually("the Mac's silence") { picker.notice != nil }
        XCTAssertEqual(picker.notice?.notice, .offline)
        await eventually("the write to be taken off") { picker.count == 7 }
        XCTAssertNil(picker.undo)
    }

    func testTheNoticeOfAWriteGoesWhenAskedTo() async throws {
        await make()
        let picker = try await loaded("req-migration")
        await backend.setFailNextTodoWrite("diskFull")
        picker.complete("todo-pick-1")
        await eventually("the Mac's answer") { picker.notice != nil }
        let notice = try XCTUnwrap(picker.notice)
        picker.dismissNotice(notice.id + 1)
        XCTAssertNotNil(picker.notice, "another note's timer")
        picker.dismissNotice(notice.id)
        XCTAssertNil(picker.notice)
        picker.complete("todo-pick-2")
        picker.clearTransient()
        XCTAssertNil(picker.undo, "what was only for a moment is not there when the page comes back")
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
