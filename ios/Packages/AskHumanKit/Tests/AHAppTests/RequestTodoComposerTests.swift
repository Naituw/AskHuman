import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

/// The "Add todo…" sheet of a request page against the mock Macs (docs/specs/todo-quick-add.md §4.2, §5.3): which
/// project and Mac the todo goes to, when it can be sent, and what becomes of it.
@MainActor
final class RequestTodoComposerTests: XCTestCase {
    private var backend: MockBackend!
    private var app: AppStore!

    private let home = MockData.homeMacId
    private let office = MockData.officeMacId

    private func make(_ scenario: MockBackend.Scenario = .demo) async {
        backend = MockBackend(scenario: scenario, latency: 0)
        let defaults = UserDefaults(suiteName: "ah-requesttodo-test-" + UUID().uuidString)!
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        await app.start()
        await eventually("the Macs and the inbox") {
            self.app.state.foreground && self.app.state.macs.count == 2 && !self.app.state.inbox.isEmpty
        }
    }

    private func item(_ requestId: String) throws -> InboxItem {
        try XCTUnwrap(app.state.inbox.first { $0.requestId == requestId })
    }

    private func sheet(of requestId: String) throws -> RequestTodoComposer {
        try XCTUnwrap(app.todoComposer(for: try item(requestId)), "\(requestId) names a project")
    }

    /// The sheet of the demo's migration question: project "HumanInLoop" of the Home MacBook Pro, which has three
    /// requests waiting in the demo (the permission card, the migration question and the what's-next).
    private func migrationSheet() throws -> RequestTodoComposer { try sheet(of: "req-migration") }

    /// What the Mac holds for the project of `composer`, oldest first.
    private func queue(_ composer: RequestTodoComposer) async throws -> [String] {
        try await backend.todoList(macId: composer.macId, projectId: composer.projectId).pending.map(\.text)
    }

    private func goOffline(_ mac: String) async {
        await backend.setLink(.macOffline, for: mac)
        await eventually("the Mac to be seen going") { self.app.state.macs.first { $0.id == mac }?.link == .macOffline }
    }

    /// Both requests the demo has waiting on the Office iMac (project "relay") are over.
    private func finishTheOfficeRequests() async {
        await backend.simulateFinishedElsewhere(requestId: "req-select", macId: office)
        await backend.simulateFinishedElsewhere(requestId: "req-stop", macId: office)
        await eventually("the Office requests to be gone") { self.app.state.inbox.allSatisfy { $0.macId != self.office } }
    }

    // MARK: the target

    func testTheTargetIsTheRequestsOwnProjectOnTheMacItCameFrom() async throws {
        await make()
        let composer = try migrationSheet()
        XCTAssertEqual(composer.macId, home)
        XCTAssertEqual(composer.macName, "Home MacBook Pro")
        XCTAssertEqual(composer.projectName, "HumanInLoop")
        XCTAssertEqual(composer.projectId, TodoProjectId.of(key: "proj-humaninloop"), "the id the Mac makes of the project key")
        XCTAssertNil(composer.unavailable)
        XCTAssertNil(composer.failure)
        XCTAssertEqual(composer.phase, .editing)

        let other = try sheet(of: "req-select")
        XCTAssertEqual(other.macId, office)
        XCTAssertEqual(other.projectName, "relay")
        XCTAssertEqual(other.projectId, TodoProjectId.of(key: "proj-relay"))
    }

    func testEveryKindOfRequestWithAProjectHasOne() async throws {
        await make()
        for requestId in ["req-migration", "req-permission", "req-stop", "req-next", "req-select"] {
            _ = try sheet(of: requestId)
        }
    }

    func testARequestWithoutAProjectHasNoComposer() async throws {
        await make()
        var nameless = try item("req-migration")
        XCTAssertTrue(nameless.hasProject)
        nameless.summary.project = ProjectRef(key: "", name: "")
        XCTAssertFalse(nameless.hasProject)
        XCTAssertNil(app.todoComposer(for: nameless))
        nameless.summary.project = ProjectRef(key: "  \n", name: "")
        XCTAssertFalse(nameless.hasProject)
        XCTAssertNil(app.todoComposer(for: nameless), "a blank key names no project either")
    }

    func testAProjectWithoutANameIsCalledByItsDirectory() async throws {
        await make()
        var unnamed = try item("req-migration")
        unnamed.summary.project = ProjectRef(key: "/Users/me/Developer/Sandbox", name: " ")
        XCTAssertEqual(try XCTUnwrap(app.todoComposer(for: unnamed)).projectName, "Sandbox")
    }

    // MARK: what stays

    func testTheTextStaysWithTheRequestWhileItIsAround() async throws {
        await make()
        let composer = try migrationSheet()
        composer.text = "Cover the rollback path"
        let request = try item("req-migration")
        XCTAssertTrue(app.todoComposer(for: request) === composer, "the same sheet is reused while the request is around")
        app.closed(id: request.id)
        XCTAssertEqual(try migrationSheet().text, "Cover the rollback path", "leaving the page keeps it")
        XCTAssertEqual(try sheet(of: "req-next").text, "", "another request has its own text")
    }

    func testTheSheetGoesWhenTheRequestEnds() async throws {
        await make()
        let before = try migrationSheet()
        before.text = "Gone with the request"
        let request = try item("req-migration")

        await backend.simulateFinishedElsewhere(requestId: "req-migration")
        await eventually("the request to be gone") { self.app.item(id: request.id) == nil }
        await eventually("its sheet to be dropped") { self.app.todoComposer(for: request) !== before }
        XCTAssertEqual(app.todoComposer(for: request)?.text, "", "a new sheet starts empty")
    }

    func testTheSheetIsKeptWhileTheFinishedRequestsPageIsOpen() async throws {
        await make()
        let composer = try migrationSheet()
        composer.text = "Still being typed"
        let request = try item("req-migration")
        app.inboxPath = [request.id]

        await backend.simulateFinishedElsewhere(requestId: "req-migration")
        await eventually("the request to be gone") { self.app.item(id: request.id) == nil }
        XCTAssertEqual(app.inboxPath, [request.id], "the page stays, with its notice")
        XCTAssertTrue(app.todoComposer(for: request) === composer, "the page is still open: so is what it holds")
        XCTAssertEqual(composer.text, "Still being typed")
    }

    // MARK: when it cannot be sent

    func testAMacThatIsOutOfReachCannotBeWrittenTo() async throws {
        await make()
        let composer = try migrationSheet()
        composer.text = "An idea"
        XCTAssertTrue(composer.canSend)

        await goOffline(home)
        XCTAssertEqual(composer.unavailable, .macOffline(mac: "Home MacBook Pro"))
        XCTAssertFalse(composer.canSend)
        let sent = await composer.send()
        XCTAssertFalse(sent)
        XCTAssertEqual(composer.text, "An idea", "nothing was lost")
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty)

        await backend.setLink(.online, for: home)
        await eventually("the Mac to be seen back") { composer.unavailable == nil }
        XCTAssertTrue(composer.canSend)
    }

    func testAMacThatIsNotPairedAnyMoreCountsAsOutOfReach() async throws {
        await make()
        let composer = try migrationSheet()
        composer.update(mac: nil)
        XCTAssertEqual(composer.unavailable, .macOffline(mac: "Home MacBook Pro"), "it is still called what it was")
    }

    func testAMacWhoseAskHumanPredatesTodosHasToBeUpdated() async throws {
        await make(.outdated)
        let composer = try migrationSheet()
        composer.text = "An idea"
        XCTAssertEqual(composer.unavailable, .macNeedsUpdate(mac: "Home MacBook Pro"))
        XCTAssertFalse(composer.canSend)
        let sent = await composer.send()
        XCTAssertFalse(sent)
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty)
    }

    func testTextThatIsTooLongIsHeldBackAtTheLimit() async throws {
        await make()
        let composer = try migrationSheet()
        composer.text = String(repeating: "x", count: Wire.todoTextMaxChars)
        XCTAssertNil(composer.unavailable)
        XCTAssertTrue(composer.canSend)

        composer.text += "x"
        XCTAssertEqual(composer.unavailable, .tooLong(count: Wire.todoTextMaxChars + 1, limit: Wire.todoTextMaxChars))
        XCTAssertFalse(composer.canSend)
    }

    func testNothingIsSentForBlankText() async throws {
        await make()
        let composer = try migrationSheet()
        for blank in ["", "   ", "\n \n"] {
            composer.text = blank
            XCTAssertFalse(composer.canSend)
            let sent = await composer.send()
            XCTAssertFalse(sent)
        }
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty)
    }

    func testTheNameOfTheMacIsTheOneItHasNow() async throws {
        await make()
        let composer = try migrationSheet()
        var renamed = try XCTUnwrap(app.state.macs.first { $0.id == home })
        renamed.name = "Study MacBook"
        composer.update(mac: renamed)
        XCTAssertEqual(composer.macName, "Study MacBook")
        composer.update(mac: nil)
        XCTAssertEqual(composer.unavailable, .macOffline(mac: "Study MacBook"), "a Mac that is gone is called what it was last")
    }

    func testTheSheetFollowsTheMacComingAndGoingWithoutBeingAsked() async throws {
        await make()
        let composer = try migrationSheet()
        await goOffline(home)
        XCTAssertNotNil(composer.unavailable)
        await backend.setLink(.online, for: home)
        await eventually("the Mac to be seen back") { composer.unavailable == nil }
    }

    // MARK: sending

    func testAddingWritesOneTodoWithoutALightningBoltAndClearsTheText() async throws {
        await make()
        let composer = try migrationSheet()
        composer.text = "  Write the rollback test \n"
        let sent = await composer.send()
        XCTAssertTrue(sent)
        XCTAssertEqual(composer.text, "")
        XCTAssertNil(composer.failure)
        XCTAssertEqual(composer.phase, .editing)

        let ops = await backend.todoOps
        XCTAssertEqual(ops.count, 1)
        let op = try XCTUnwrap(ops.first)
        XCTAssertEqual(op.op, "add")
        XCTAssertEqual(op.id, TodoProjectId.of(key: "proj-humaninloop"))
        XCTAssertEqual(op.text, "Write the rollback test", "the text arrives trimmed")
        XCTAssertNotEqual(op.auto, true, "no ⚡: the todo only waits in the queue")
        let stored = try await queue(composer)
        XCTAssertEqual(stored, ["Write the rollback test"])
    }

    func testTheMacOfTheRequestGetsTheTodoAndNoOtherMacDoes() async throws {
        await make()
        let composer = try sheet(of: "req-select")
        composer.text = "Pin the relay version"
        let sent = await composer.send()
        XCTAssertTrue(sent)
        let stored = try await queue(composer)
        XCTAssertEqual(stored, ["Pin the relay version"])
        let onHome = try await backend.todoList(macId: home, projectId: composer.projectId)
        XCTAssertFalse(onHome.found, "the other Mac has not heard of it")
    }

    func testTodosAreAppendedAtTheEndOfTheQueue() async throws {
        await make()
        let composer = try migrationSheet()
        for text in ["First", "Second", "Third"] {
            composer.text = text
            let sent = await composer.send()
            XCTAssertTrue(sent)
        }
        let stored = try await queue(composer)
        XCTAssertEqual(stored, ["First", "Second", "Third"])
    }

    func testAProjectOnlyAWaitingRequestKnowsIsStartedOnTheMac() async throws {
        await make()
        let composer = try migrationSheet()
        let before = try await backend.todoList(macId: home, projectId: composer.projectId)
        XCTAssertFalse(before.found, "no list of the Mac names it yet")
        let rowsBefore = try await backend.todoProjects(macId: home).projects
        XCTAssertFalse(rowsBefore.contains { $0.id == composer.projectId })

        composer.text = "The first todo of the project"
        let sent = await composer.send()
        XCTAssertTrue(sent)
        let after = try await backend.todoList(macId: home, projectId: composer.projectId)
        XCTAssertTrue(after.found)
        XCTAssertEqual(after.name, "HumanInLoop")
        XCTAssertEqual(after.pending.map(\.text), ["The first todo of the project"])

        // The Mac has todos of it now, so the Todos tab finds the project too (once: not for every todo).
        composer.text = "The second"
        let again = await composer.send()
        XCTAssertTrue(again)
        let rows = try await backend.todoProjects(macId: home).projects.filter { $0.id == composer.projectId }
        XCTAssertEqual(rows.map(\.name), ["HumanInLoop"])
        XCTAssertEqual(rows.map(\.pending), [2])
    }

    func testAMacOnlyKnowsTheProjectsOfItsOwnRequests() async throws {
        await make()
        let operation = TodoMutation.add(localId: "x", text: "Not yours", auto: false)
            .operation(clientMsgId: "x", projectId: TodoProjectId.of(key: "proj-humaninloop"))
        let ack = try await backend.todoApply(macId: office, operation: operation)
        XCTAssertEqual(ack.status, "notFound", "the Office iMac has no request of that project")
        XCTAssertEqual(ack.reason, "unknownProject")
    }

    func testAllKindsOfRequestsAddToTheirProject() async throws {
        await make()
        let confirm = try sheet(of: "req-permission")
        let ask = try migrationSheet()
        XCTAssertEqual(confirm.projectId, ask.projectId, "both belong to HumanInLoop on the Home MacBook Pro")
        confirm.text = "From a permission card"
        ask.text = "From a question"
        let first = await confirm.send()
        let second = await ask.send()
        XCTAssertTrue(first)
        XCTAssertTrue(second)
        let stored = try await queue(ask)
        XCTAssertEqual(stored, ["From a permission card", "From a question"])
    }

    // MARK: when it goes wrong

    func testALostAnswerIsAskedForAgainUnderTheSameIdAndTheTodoIsStoredOnce() async throws {
        await make()
        let composer = try migrationSheet()
        composer.text = "Only once please"
        await backend.setLoseNextTodoAnswers(1)
        let sent = await composer.send()
        XCTAssertTrue(sent, "the second ask is answered")

        let ops = await backend.todoOps
        XCTAssertEqual(ops.count, 2)
        XCTAssertEqual(Set(ops.map(\.clientMsgId)).count, 1, "both asks carry the same id")
        let stored = try await queue(composer)
        XCTAssertEqual(stored, ["Only once please"])
    }

    func testWhenNoAnswerComesTwiceTheTextStaysAndSendingAgainRepeatsTheSameWrite() async throws {
        await make()
        let composer = try migrationSheet()
        composer.text = "Maybe there already"
        await backend.setLoseNextTodoAnswers(2)
        var sent = await composer.send()
        XCTAssertFalse(sent)
        XCTAssertEqual(composer.failure, .timedOut(mac: "Home MacBook Pro"))
        XCTAssertEqual(composer.text, "Maybe there already")
        XCTAssertEqual(composer.phase, .editing)
        let heardOf = try await queue(composer)
        XCTAssertEqual(heardOf.count, 1, "the Mac did store it, the phone only never heard")

        sent = await composer.send()
        XCTAssertTrue(sent)
        XCTAssertNil(composer.failure)
        XCTAssertEqual(composer.text, "")
        let ops = await backend.todoOps
        XCTAssertEqual(Set(ops.map(\.clientMsgId)).count, 1, "every ask is the same write")
        let stored = try await queue(composer)
        XCTAssertEqual(stored, ["Maybe there already"], "and it is still there once")
    }

    func testChangedTextAfterALostAnswerIsAnotherWrite() async throws {
        await make()
        let composer = try migrationSheet()
        composer.text = "First wording"
        await backend.setLoseNextTodoAnswers(2)
        _ = await composer.send()
        composer.text = "Second wording"
        let sent = await composer.send()
        XCTAssertTrue(sent)
        let ops = await backend.todoOps
        XCTAssertEqual(Set(ops.map(\.clientMsgId)).count, 2, "another text is another todo")
    }

    func testAMacThatCannotStoreItLeavesTheTextAndSaysSo() async throws {
        await make()
        let composer = try migrationSheet()
        composer.text = "Will not be saved"
        await backend.setFailNextTodoWrite("busy")
        var sent = await composer.send()
        XCTAssertFalse(sent)
        XCTAssertEqual(composer.failure, .notSaved)
        XCTAssertEqual(composer.text, "Will not be saved")

        sent = await composer.send()
        XCTAssertTrue(sent, "the next try goes through")
        XCTAssertNil(composer.failure, "an old failure does not outlive the next try")
    }

    func testAMacThatDoesNotKnowTheProjectAnyMoreRefusesAndTheTextStays() async throws {
        await make()
        let composer = try sheet(of: "req-select")
        // Nothing is waiting on the Office iMac now, and it has never stored a todo of the project.
        await finishTheOfficeRequests()

        composer.text = "Too late"
        let sent = await composer.send()
        XCTAssertFalse(sent)
        XCTAssertEqual(composer.failure, .refused("unknownProject"))
        XCTAssertEqual(composer.text, "Too late")
        XCTAssertEqual(composer.phase, .editing)
    }

    func testAFinishedRequestStillTakesTodosForAProjectTheMacKeeps() async throws {
        await make()
        let composer = try sheet(of: "req-select")
        composer.text = "Before it ended"
        let first = await composer.send()
        XCTAssertTrue(first)

        // The Mac has the project among its todos now, whether or not a request still waits on it.
        await finishTheOfficeRequests()
        composer.text = "After it ended"
        let second = await composer.send()
        XCTAssertTrue(second)
        let stored = try await queue(composer)
        XCTAssertEqual(stored, ["Before it ended", "After it ended"])
    }

    func testWhatStandsInTheWayNowIsNotHiddenByAnOldFailure() async throws {
        await make()
        let composer = try migrationSheet()
        composer.text = "An idea"
        await backend.setFailNextTodoWrite("busy")
        _ = await composer.send()
        XCTAssertEqual(composer.failure, .notSaved)

        await goOffline(home)
        XCTAssertEqual(composer.unavailable, .macOffline(mac: "Home MacBook Pro"), "the sheet says this first")
    }
}
