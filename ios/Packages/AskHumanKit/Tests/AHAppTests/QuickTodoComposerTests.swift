import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

/// The quick-add of the developer mode against the mock Macs (docs/specs/ios-developer-mode.md §6): which
/// project the todo goes to, when it can be sent, and what becomes of it.
@MainActor
final class QuickTodoComposerTests: XCTestCase {
    private var backend: MockBackend!
    private var app: AppStore!
    private var defaults: UserDefaults!
    private var composer: QuickTodoComposer { app.developer.quickTodo }

    private let home = MockData.homeMacId
    private let office = MockData.officeMacId
    private let targetKey = "ah.dev.quickTodoTarget"

    private func freshDefaults() -> UserDefaults {
        UserDefaults(suiteName: "ah-quicktodo-test-" + UUID().uuidString)!
    }

    /// `existing`: the defaults of an earlier run (what the app remembered).
    private func make(_ scenario: MockBackend.Scenario = .demo, defaults existing: UserDefaults? = nil) async {
        backend = MockBackend(scenario: scenario, latency: 0)
        defaults = existing ?? freshDefaults()
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        await app.start()
        await eventually("the Macs") { self.app.state.foreground && self.app.state.macs.count == 2 }
    }

    private func state(of mac: String) -> QuickTodoGroup.State? {
        composer.groups.first { $0.macId == mac }?.state
    }

    private func projects(of mac: String) -> [TodoListedProject] {
        if case .projects(let rows)? = state(of: mac) { return rows }
        return []
    }

    /// Picks the Office iMac's "AskHuman · work".
    private func chooseOfficeWork() async {
        await composer.loadProjects()
        guard let project = projects(of: office).first(where: { $0.id == MockData.todoOfficeWorkProject }) else {
            return XCTFail("the Office iMac lists no AskHuman · work")
        }
        composer.choose(project)
    }

    private func pending(_ project: String = MockData.todoOfficeWorkProject, on mac: String? = nil) async throws -> [TodoItem] {
        try await backend.todoList(macId: mac ?? office, projectId: project).pending
    }

    private func stored(_ target: QuickTodoTarget, in defaults: UserDefaults) throws {
        defaults.set(try JSONEncoder().encode(target), forKey: targetKey)
    }

    // MARK: the target

    func testWithoutAProjectNothingCanBeSent() async {
        await make()
        XCTAssertNil(composer.target)
        composer.text = "An idea"
        XCTAssertEqual(composer.unavailable, .noProject)
        XCTAssertFalse(composer.canSend)
        let sent = await composer.send()
        XCTAssertNil(sent)
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty)
    }

    func testTheChosenProjectIsRememberedAcrossLaunches() async throws {
        await make()
        await chooseOfficeWork()
        XCTAssertEqual(composer.target?.projectId, MockData.todoOfficeWorkProject)
        XCTAssertEqual(composer.target?.projectHint, "work")
        XCTAssertEqual(composer.target?.macName, "Office iMac")

        let defaults = self.defaults!
        await make(defaults: defaults)
        XCTAssertEqual(composer.target?.projectName, "AskHuman")
        XCTAssertEqual(composer.target?.macId, office)
        XCTAssertNil(composer.unavailable, "a remembered project is ready to take a todo")
    }

    func testAProjectOfAMacThatIsNotPairedCountsAsNoneAndComesBackWithTheMac() async throws {
        let defaults = freshDefaults()
        try stored(QuickTodoTarget(macId: "gone-mac", macName: "Old Mac", projectId: "p1", projectName: "Kept"), in: defaults)
        await make(defaults: defaults)
        XCTAssertNil(composer.target)
        XCTAssertEqual(composer.unavailable, .noProject)

        var returned = app.state.macs[0]
        returned.id = "gone-mac"
        returned.name = "Old Mac"
        composer.update(macs: app.state.macs + [returned])
        XCTAssertEqual(composer.target?.projectName, "Kept", "paired again: the project is back")

        composer.update(macs: app.state.macs)
        XCTAssertNil(composer.target)
    }

    func testTheMacsNameIsTheOneItHasNowNotTheOneRememberedWithTheProject() async throws {
        let defaults = freshDefaults()
        try stored(QuickTodoTarget(macId: home, macName: "Before the rename", projectId: "p", projectName: "AskHuman"), in: defaults)
        await make(defaults: defaults)
        XCTAssertEqual(composer.target?.macName, "Home MacBook Pro")
    }

    // MARK: whether it can be sent

    func testATargetMacThatIsOfflineOrTooOldBlocksTheSendAndSaysWhich() async throws {
        await make()
        await chooseOfficeWork()
        composer.text = "An idea"
        XCTAssertNil(composer.unavailable)
        XCTAssertTrue(composer.canSend)

        await backend.setLink(.macOffline, for: office)
        await eventually("the Mac to be seen going") { self.composer.unavailable != nil }
        XCTAssertEqual(composer.unavailable, .macOffline(mac: "Office iMac"))
        XCTAssertFalse(composer.canSend)
        let sent = await composer.send()
        XCTAssertNil(sent)
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty, "nothing is asked of a Mac that is known to be gone")

        let before = freshDefaults()
        try stored(QuickTodoTarget(macId: home, macName: "Home MacBook Pro", projectId: "p", projectName: "AskHuman"), in: before)
        await make(.outdated, defaults: before)
        composer.text = "An idea"
        XCTAssertEqual(composer.unavailable, .macNeedsUpdate(mac: "Home MacBook Pro"))
        XCTAssertFalse(composer.canSend)
    }

    func testTheTextIsCheckedBeforeAnythingIsSent() async {
        await make()
        await chooseOfficeWork()

        composer.text = "  \n "
        XCTAssertNil(composer.unavailable, "nothing typed needs no explanation")
        XCTAssertFalse(composer.canSend)

        composer.text = String(repeating: "x", count: Wire.todoTextMaxChars)
        XCTAssertTrue(composer.canSend)

        composer.text = String(repeating: "x", count: Wire.todoTextMaxChars + 1)
        XCTAssertEqual(composer.unavailable, .tooLong(count: Wire.todoTextMaxChars + 1, limit: Wire.todoTextMaxChars))
        XCTAssertFalse(composer.canSend)

        composer.text = " " + String(repeating: "x", count: Wire.todoTextMaxChars) + "\n"
        XCTAssertTrue(composer.canSend, "the space around the text does not count")
    }

    // MARK: sending

    func testASendAddsTheTrimmedTextAsAPlainTodoAndClearsTheField() async throws {
        await make()
        await chooseOfficeWork()
        let before = try await pending().count

        composer.text = "  Try the new idea \n"
        let target = await composer.send()

        XCTAssertEqual(target?.projectName, "AskHuman")
        XCTAssertEqual(composer.text, "")
        XCTAssertNil(composer.failure)
        XCTAssertEqual(composer.phase, .editing)

        let ops = await backend.todoOps
        XCTAssertEqual(ops.count, 1)
        XCTAssertEqual(ops[0].op, "add")
        XCTAssertEqual(ops[0].id, MockData.todoOfficeWorkProject)
        XCTAssertEqual(ops[0].text, "Try the new idea")
        XCTAssertNil(ops[0].auto, "a todo added from here never starts an Agent")

        let todos = try await pending()
        XCTAssertEqual(todos.count, before + 1)
        XCTAssertEqual(todos.last?.text, "Try the new idea")
        XCTAssertEqual(todos.last?.auto, false)
    }

    func testAnAnswerThatGetsLostIsAskedForAgainUnderTheSameIdAndTheTodoIsAddedOnce() async throws {
        await make()
        await chooseOfficeWork()
        await backend.setLoseNextTodoAnswers(1)

        composer.text = "Lost once"
        let target = await composer.send()
        XCTAssertNotNil(target)

        let ops = await backend.todoOps
        XCTAssertEqual(ops.count, 2)
        XCTAssertEqual(ops[0].clientMsgId, ops[1].clientMsgId)
        let added = try await pending().filter { $0.text == "Lost once" }
        XCTAssertEqual(added.count, 1)
    }

    func testWhenNoAnswerComesTwiceTheTextStaysAndTappingAddAgainStillAddsOnce() async throws {
        await make()
        await chooseOfficeWork()
        await backend.setLoseNextTodoAnswers(2)

        composer.text = "Lost twice"
        let first = await composer.send()
        XCTAssertNil(first)
        XCTAssertEqual(composer.failure, .timedOut(mac: "Office iMac"))
        XCTAssertEqual(composer.text, "Lost twice")
        XCTAssertTrue(composer.canSend, "it can be tried again")
        var count = try await pending().filter { $0.text == "Lost twice" }.count
        XCTAssertEqual(count, 1, "the Mac did store it; only its answer was lost")

        let second = await composer.send()
        XCTAssertNotNil(second)
        XCTAssertNil(composer.failure)
        let ops = await backend.todoOps
        XCTAssertEqual(Set(ops.map(\.clientMsgId)).count, 1, "every try carried the same id")
        count = try await pending().filter { $0.text == "Lost twice" }.count
        XCTAssertEqual(count, 1, "and the Mac stored it once")
    }

    func testChangingTheTextAfterALostAnswerSendsItUnderANewId() async throws {
        await make()
        await chooseOfficeWork()
        await backend.setLoseNextTodoAnswers(2)

        composer.text = "First wording"
        _ = await composer.send()
        composer.text = "Second wording"
        _ = await composer.send()

        let ops = await backend.todoOps
        XCTAssertEqual(ops.count, 3)
        XCTAssertEqual(ops[0].clientMsgId, ops[1].clientMsgId)
        XCTAssertNotEqual(ops[2].clientMsgId, ops[0].clientMsgId)
    }

    func testAMacThatCouldNotStoreItLeavesTheTextAndTheNextTryIsANewSend() async throws {
        await make()
        await chooseOfficeWork()
        await backend.setFailNextTodoWrite("persist")

        composer.text = "Hard to store"
        let failed = await composer.send()
        XCTAssertNil(failed)
        XCTAssertEqual(composer.failure, .notSaved)
        XCTAssertEqual(composer.text, "Hard to store")
        XCTAssertEqual(composer.phase, .editing)

        let retried = await composer.send()
        XCTAssertNotNil(retried)
        XCTAssertNil(composer.failure)
        let ops = await backend.todoOps
        XCTAssertEqual(ops.count, 2)
        XCTAssertNotEqual(ops[0].clientMsgId, ops[1].clientMsgId)
        let added = try await pending().filter { $0.text == "Hard to store" }
        XCTAssertEqual(added.count, 1)
    }

    func testAMacThatDropsOutAsItIsAskedIsUnreachableAndNothingWasSent() async throws {
        await make()
        await chooseOfficeWork()
        await backend.setUnreachable([office])

        composer.text = "Too late"
        let sent = await composer.send()
        XCTAssertNil(sent)
        XCTAssertEqual(composer.failure, .unreachable(mac: "Office iMac"))
        XCTAssertEqual(composer.text, "Too late")
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty)
    }

    func testAProjectThatIsGoneIsForgottenAndTheTextStays() async throws {
        let defaults = freshDefaults()
        try stored(QuickTodoTarget(macId: home, macName: "Home MacBook Pro", projectId: "no-such-project", projectName: "Old"), in: defaults)
        await make(defaults: defaults)
        XCTAssertNotNil(composer.target)

        composer.text = "For a project that went"
        let sent = await composer.send()
        XCTAssertNil(sent)
        XCTAssertEqual(composer.failure, .projectGone(mac: "Home MacBook Pro"))
        XCTAssertEqual(composer.text, "For a project that went")
        XCTAssertNil(composer.target, "a project that is not there cannot stay the target")
        XCTAssertEqual(composer.unavailable, .noProject)

        let next = QuickTodoComposer(backend: backend, defaults: defaults)
        next.update(macs: app.state.macs)
        XCTAssertNil(next.target, "and it is gone from what is remembered")
    }

    func testChoosingAProjectClearsTheLastFailure() async throws {
        await make()
        await chooseOfficeWork()
        await backend.setFailNextTodoWrite("persist")
        composer.text = "x"
        _ = await composer.send()
        XCTAssertNotNil(composer.failure)

        let other = projects(of: office).first { $0.id == MockData.todoOfficePlayProject }
        composer.choose(try XCTUnwrap(other))
        XCTAssertNil(composer.failure)
        XCTAssertEqual(composer.target?.projectHint, "playground")
    }

    // MARK: the project list

    func testTheListHasEveryMacThatIsOnlineAndKnowsTodosWithTheirProjectsInTheirOrder() async {
        await make()
        await composer.loadProjects()
        XCTAssertEqual(composer.groups.map(\.macId), [home, office])
        XCTAssertEqual(composer.groups.map(\.macName), ["Home MacBook Pro", "Office iMac"])

        XCTAssertEqual(
            projects(of: office).map(\.id),
            [MockData.todoOfficeWorkProject, MockData.todoOfficePlayProject, MockData.todoOfficeSandboxProject]
        )
        XCTAssertEqual(projects(of: office).map(\.hint), ["work", "playground", nil], "same name, told apart by the hint")
        XCTAssertTrue(projects(of: office).allSatisfy { $0.macId == office && $0.macName == "Office iMac" })
        XCTAssertTrue(projects(of: home).contains { $0.name == "notes" && $0.scope == "cloudOnly" }, "iCloud-only projects can be chosen too")
    }

    func testTheSameICloudProjectIsListedUnderEveryMacThatSyncs() async {
        await make(.bothSync)
        await composer.loadProjects()
        let syncedAskHuman = { (mac: String) in self.projects(of: mac).filter { $0.name == "AskHuman" && $0.row.repo != nil } }
        XCTAssertEqual(syncedAskHuman(home).count, 1)
        XCTAssertEqual(syncedAskHuman(office).count, 1, "not merged: choosing it under a Mac sends through that Mac")
    }

    func testMacsThatCannotBeAskedAreListedWithTheirReason() async {
        await make(.offline)
        await composer.loadProjects()
        XCTAssertEqual(state(of: home), .offline)
        XCTAssertEqual(state(of: office), .offline)

        await make(.outdated)
        await composer.loadProjects()
        XCTAssertEqual(state(of: home), .needsUpdate)
        XCTAssertEqual(state(of: office), .needsUpdate)

        await make()
        await backend.setLink(.macOffline, for: office)
        await eventually("the Mac to be seen going") { self.app.state.macs.first { $0.id == self.office }?.link == .macOffline }
        await composer.loadProjects()
        XCTAssertEqual(state(of: office), .offline)
        XCTAssertFalse(projects(of: home).isEmpty, "the Mac that is there is asked")
    }

    func testAMacThatDoesNotAnswerIsMarkedSoAndCanBeAskedAgain() async {
        await make()
        await backend.setLink(.macOffline, for: office)
        await eventually("the Mac to be seen going") { self.app.state.macs.first { $0.id == self.office }?.link == .macOffline }

        await backend.setLoseNextTodoProjectAnswers(1)
        await composer.loadProjects()
        XCTAssertEqual(state(of: home), .noAnswer)
        XCTAssertEqual(state(of: office), .offline)

        await composer.retry(macId: home)
        XCTAssertFalse(projects(of: home).isEmpty)
    }

    func testTheSearchKeepsTheMatchingProjectsAndTheMacsThatHaveThem() async {
        await make()
        await composer.loadProjects()

        composer.query = "playground"
        XCTAssertEqual(composer.visibleGroups.map(\.macId), [office])
        if case .projects(let rows)? = composer.visibleGroups.first?.state {
            XCTAssertEqual(rows.map(\.id), [MockData.todoOfficePlayProject])
        } else {
            XCTFail("the Office iMac's matches")
        }

        composer.query = "no such project anywhere"
        XCTAssertTrue(composer.visibleGroups.isEmpty)

        composer.query = "  "
        XCTAssertEqual(composer.visibleGroups.count, 2, "a blank search is no search")
    }

    func testWhileSearchingMacsWithoutProjectsAreNotShown() async {
        await make(.offline)
        await composer.loadProjects()
        XCTAssertEqual(composer.visibleGroups.count, 2, "without a search the reasons are listed")
        composer.query = "anything"
        XCTAssertTrue(composer.visibleGroups.isEmpty)
    }

    func testAListThatIsOpenFollowsAMacComingOnlineAndAClosedOneDoesNot() async {
        await make(.offline)
        await composer.loadProjects()
        XCTAssertEqual(state(of: home), .offline)

        await backend.setLink(.online, for: home)
        await eventually("the Mac to be listed") { !self.projects(of: self.home).isEmpty }

        composer.closeProjects()
        await backend.setLink(.online, for: office)
        await eventually("the Mac to be seen") { self.app.state.macs.first { $0.id == self.office }?.link == .online }
        try? await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(state(of: office), .offline, "nobody is looking: nothing was asked")
    }
}
