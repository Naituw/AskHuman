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
    /// The repository both Macs know the demo's "AskHuman" by, once the Office iMac syncs too.
    private let askHumanRepo = "github.com/naituw/askhuman"

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

    private func source(_ id: TodoSource) -> QuickTodoSource? {
        composer.sources.first { $0.source == id }
    }

    private func rows(in id: TodoSource) -> [TodoListedProject] {
        if case .projects(let rows)? = source(id)?.state { return rows }
        return []
    }

    /// Picks the Office iMac's "AskHuman · work", a project of a Mac that does not sync.
    private func chooseOfficeWork() async {
        await composer.loadProjects()
        guard let project = rows(in: .mac(office)).first(where: { $0.id == MockData.todoOfficeWorkProject }) else {
            return XCTFail("the Office iMac lists no AskHuman · work")
        }
        composer.choose(project, in: .mac(office))
        composer.closeProjects()
    }

    /// Picks iCloud's "AskHuman", the project with a repository that every Mac that syncs has.
    private func chooseICloudAskHuman() async {
        await composer.loadProjects()
        guard let project = rows(in: .icloud).first(where: { $0.repo == askHumanRepo }) else {
            return XCTFail("iCloud lists no AskHuman")
        }
        composer.choose(project, in: .icloud)
        composer.closeProjects()
    }

    private func pending(_ project: String = MockData.todoOfficeWorkProject, on mac: String? = nil) async throws -> [TodoItem] {
        try await backend.todoList(macId: mac ?? office, projectId: project).pending
    }

    private func stored(_ target: QuickTodoTarget, in defaults: UserDefaults) throws {
        defaults.set(try JSONEncoder().encode(target), forKey: targetKey)
    }

    private func goOffline(_ mac: String) async {
        await backend.setLink(.macOffline, for: mac)
        await eventually("the Mac to be seen going") { self.app.state.macs.first { $0.id == mac }?.link == .macOffline }
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
        XCTAssertFalse(composer.target?.viaICloud ?? true, "a project of a Mac that does not sync is that Mac's")

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

    func testAnICloudProjectIsChosenAsICloudsAndSurvivesTheMacItWasChosenThrough() async throws {
        await make(.bothSync)
        await chooseICloudAskHuman()
        let target = try XCTUnwrap(composer.target)
        XCTAssertTrue(target.viaICloud)
        XCTAssertEqual(target.icloudRepo, askHumanRepo)
        XCTAssertEqual(target.macId, home, "it was listed by the Mac that serves iCloud")
        XCTAssertEqual(target.projectId, MockData.todoAskHumanProject)
        XCTAssertEqual(composer.currentSource, .icloud)

        let defaults = self.defaults!
        await make(.bothSync, defaults: defaults)
        XCTAssertEqual(composer.target?.icloudRepo, askHumanRepo, "what is kept is the repository, not a Mac's id")
        XCTAssertNil(composer.unavailable)

        // The Mac it was chosen through is not paired any more: iCloud's project is still there.
        composer.update(macs: app.state.macs.filter { $0.id == office })
        XCTAssertEqual(composer.target?.projectName, "AskHuman")
        XCTAssertNil(composer.unavailable)
        composer.update(macs: [])
        XCTAssertNil(composer.target, "with no Mac at all there is nowhere to send it")
    }

    func testATargetKeptByAnEarlierVersionIsAProjectOfThatMac() async throws {
        let defaults = freshDefaults()
        let old = #"{"macId":"\#(home)","macName":"Home MacBook Pro","projectId":"\#(MockData.todoAskHumanProject)","projectName":"AskHuman"}"#
        defaults.set(Data(old.utf8), forKey: targetKey)
        await make(defaults: defaults)
        XCTAssertEqual(composer.target?.projectName, "AskHuman")
        XCTAssertEqual(composer.target?.viaICloud, false)
        XCTAssertEqual(composer.currentSource, .mac(home), "listed under the Mac until the Todos tab learns that it syncs")
    }

    func testAProjectOnlyOneMacBehindICloudHasStaysThatMacs() async throws {
        await make(.bothSync)
        await composer.loadProjects()
        let intranet = try XCTUnwrap(rows(in: .icloud).first { $0.id == MockData.todoOfficeIntranetProject })
        XCTAssertEqual(intranet.macId, office, "the Office iMac told it")
        composer.choose(intranet, in: .icloud)
        XCTAssertFalse(composer.target?.viaICloud ?? true, "nothing of it is in iCloud")
        XCTAssertEqual(composer.target?.macId, office)
        XCTAssertEqual(composer.currentSource, .icloud, "it is listed with iCloud all the same")
        XCTAssertTrue(composer.isCurrent(intranet, in: .icloud))

        await goOffline(office)
        XCTAssertEqual(composer.unavailable, .macOffline(mac: "Office iMac"), "no other Mac has it")
    }

    // MARK: whether it can be sent

    func testATargetMacThatIsOfflineOrTooOldBlocksTheSendAndSaysWhich() async throws {
        await make()
        await chooseOfficeWork()
        composer.text = "An idea"
        XCTAssertNil(composer.unavailable)
        XCTAssertTrue(composer.canSend)

        await goOffline(office)
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

    func testAnICloudProjectNeedsAMacThatSyncsToBeReachable() async throws {
        await make()
        await chooseICloudAskHuman()
        composer.text = "An idea"
        XCTAssertNil(composer.unavailable)

        // Only the Home MacBook Pro syncs; the Office iMac, which is up, does not serve iCloud.
        await goOffline(home)
        XCTAssertEqual(composer.unavailable, .icloudOffline)
        XCTAssertFalse(composer.canSend)
        let sent = await composer.send()
        XCTAssertNil(sent)
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty, "nothing is asked of a Mac that is known to be gone")
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

    func testAnICloudProjectGoesThroughTheMacItWasChosenThrough() async throws {
        await make(.bothSync)
        await chooseICloudAskHuman()
        composer.text = "Through the first Mac"
        let target = await composer.send()
        XCTAssertNotNil(target)
        let ops = await backend.todoOps
        XCTAssertEqual(ops.map(\.id), [MockData.todoAskHumanProject])
        let todos = try await pending(MockData.todoAskHumanProject, on: home)
        XCTAssertEqual(todos.last?.text, "Through the first Mac")
    }

    func testAnICloudProjectGoesThroughAnotherMacWhenTheOneItWasChosenThroughIsGone() async throws {
        await make(.bothSync)
        await chooseICloudAskHuman()
        await goOffline(home)

        composer.text = "While the first Mac sleeps"
        XCTAssertNil(composer.unavailable, "the other Mac that syncs can take it")
        let target = await composer.send()
        XCTAssertNotNil(target)
        XCTAssertNil(composer.failure)

        // The Office iMac has its own id for the same repository, which is what the write carried.
        let ops = await backend.todoOps
        XCTAssertEqual(ops.map(\.id), [MockData.todoOfficeWorkProject])
        let todos = try await pending(MockData.todoOfficeWorkProject, on: office)
        XCTAssertEqual(todos.last?.text, "While the first Mac sleeps")
        XCTAssertEqual(composer.target?.macId, home, "what is remembered did not move")
        XCTAssertEqual(composer.target?.projectId, MockData.todoAskHumanProject)

        // The first Mac is back: it takes the next one, under its own id again.
        await backend.setLink(.online, for: home)
        await eventually("the Mac to be seen back") { self.app.state.macs.first { $0.id == self.home }?.link == .online }
        composer.text = "Back on the first Mac"
        _ = await composer.send()
        let after = await backend.todoOps
        XCTAssertEqual(after.map(\.id), [MockData.todoOfficeWorkProject, MockData.todoAskHumanProject])
    }

    func testAnICloudProjectTheOtherMacDoesNotKnowIsNotSentAnywhereWhenItsMacIsGone() async throws {
        await make(.bothSync)
        await composer.loadProjects()
        let billing = try XCTUnwrap(rows(in: .icloud).first { $0.id == MockData.todoBillingProject })
        composer.choose(billing, in: .icloud)
        await goOffline(home)

        // The Office iMac (in this mock) lists none of the other synced projects, so it has nowhere to put it.
        composer.text = "Nowhere to go"
        XCTAssertNil(composer.unavailable)
        let sent = await composer.send()
        XCTAssertNil(sent)
        XCTAssertEqual(composer.failure, .icloudUnreachable)
        XCTAssertEqual(composer.text, "Nowhere to go")
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty)
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

    func testAnICloudProjectThatIsGoneIsForgottenToo() async throws {
        let defaults = freshDefaults()
        try stored(
            QuickTodoTarget(
                macId: home, macName: "Home MacBook Pro", projectId: "no-such-project", projectName: "Old",
                icloudRepo: "github.com/nobody/old"
            ),
            in: defaults
        )
        await make(defaults: defaults)
        composer.text = "For a project that went"
        let sent = await composer.send()
        XCTAssertNil(sent)
        XCTAssertEqual(composer.failure, .projectGone(mac: nil), "it is iCloud's project that is gone, not a Mac's")
        XCTAssertNil(composer.target)
    }

    func testChoosingAProjectClearsTheLastFailure() async throws {
        await make()
        await chooseOfficeWork()
        await backend.setFailNextTodoWrite("persist")
        composer.text = "x"
        _ = await composer.send()
        XCTAssertNotNil(composer.failure)

        let other = rows(in: .mac(office)).first { $0.id == MockData.todoOfficePlayProject }
        composer.choose(try XCTUnwrap(other), in: .mac(office))
        XCTAssertNil(composer.failure)
        XCTAssertEqual(composer.target?.projectHint, "playground")
    }

    // MARK: the sources

    func testTheSourcesAreICloudAndTheMacsThatAreNotPartOfItAsTheTodosTabListsThem() async {
        await make()
        await composer.loadProjects()
        XCTAssertEqual(composer.sources.map(\.id), ["icloud", "mac:" + office], "the Home MacBook Pro syncs: it is iCloud")
        XCTAssertEqual(composer.sources[0].choice.members.map(\.name), ["Home MacBook Pro"])
        XCTAssertTrue(composer.sources.allSatisfy(\.canOpen))

        let cloud = rows(in: .icloud)
        XCTAssertEqual(cloud.map(\.name), ["AskHuman", "Billing Service", "blog", "notes", "dotfiles", "scratch"], "in the order the Mac gave them")
        XCTAssertTrue(cloud.allSatisfy { $0.macId == home && $0.macName == "Home MacBook Pro" })
        XCTAssertEqual(source(.icloud)?.servedBy, "Home MacBook Pro")
        XCTAssertTrue(cloud.contains { $0.name == "notes" && $0.scope == "cloudOnly" }, "iCloud-only projects can be chosen too")

        XCTAssertEqual(
            rows(in: .mac(office)).map(\.id),
            [MockData.todoOfficeWorkProject, MockData.todoOfficePlayProject, MockData.todoOfficeSandboxProject]
        )
        XCTAssertEqual(rows(in: .mac(office)).map(\.hint), ["work", "playground", nil], "same name, told apart by the hint")
        XCTAssertTrue(rows(in: .mac(office)).allSatisfy { $0.macId == office && $0.macName == "Office iMac" })
    }

    func testTheProjectsOfASourceAreGroupedAsTheTodosTabGroupsThem() async {
        await make()
        await composer.loadProjects()
        let cloud = composer.sections(of: .icloud)
        XCTAssertEqual(cloud.map(\.kind), [.synced, .empty, .localOnly])
        XCTAssertEqual(cloud[0].rows.map(\.name), ["AskHuman", "Billing Service", "blog", "notes"])
        XCTAssertTrue(cloud[1].collapsed, "the projects without todos stay closed until asked for")
        XCTAssertEqual(cloud[1].total, 1)
        XCTAssertEqual(cloud[2].macId, home)

        composer.emptyExpanded = true
        XCTAssertEqual(composer.sections(of: .icloud).first { $0.kind == .empty }?.rows.map(\.name), ["dotfiles"])

        XCTAssertEqual(composer.sections(of: .mac(office)).map(\.kind), [.withTodos, .recent])
        composer.closeProjects()
        XCTAssertFalse(composer.emptyExpanded, "a list that is opened again starts closed")
    }

    func testMacsThatBothSyncAreOneICloudAndTheSameProjectIsListedOnce() async throws {
        await make(.bothSync)
        await composer.loadProjects()
        XCTAssertEqual(composer.sources.map(\.id), ["icloud"], "neither Mac has a source of its own")
        XCTAssertEqual(composer.sources[0].choice.members.map(\.name), ["Home MacBook Pro", "Office iMac"])

        let cloud = rows(in: .icloud)
        XCTAssertEqual(cloud.filter { $0.name == "AskHuman" && $0.repo != nil }.count, 1, "not once for each Mac that syncs")

        // What only the Office iMac has is listed with iCloud, under that Mac.
        let office = try XCTUnwrap(cloud.first { $0.id == MockData.todoOfficeIntranetProject })
        XCTAssertEqual(office.macId, self.office)
        let sections = composer.sections(of: .icloud)
        XCTAssertEqual(sections.map(\.kind), [.synced, .empty, .localOnly, .localOnly])
        XCTAssertEqual(sections.filter { $0.kind == .localOnly }.map(\.macId), [home, self.office])
        XCTAssertEqual(sections.first { $0.kind == .empty }?.total, 2, "the Office iMac's project without todos joins the others")
    }

    func testWithOneMacPairedICloudAndTheMacAreBothListedAsOnTheTodosTab() async {
        await make()
        composer.update(macs: app.state.macs.filter { $0.id == home })
        await composer.loadProjects()
        XCTAssertEqual(composer.sources.map(\.id), ["icloud", "mac:" + home])
        XCTAssertTrue(composer.sources[0].choice.members.isEmpty, "the one Mac is not called a member")
        XCTAssertTrue(rows(in: .icloud).contains { $0.name == "notes" }, "iCloud has the project no folder is for")
        XCTAssertFalse(rows(in: .mac(home)).isEmpty)

        composer.sections(of: .mac(home)).forEach { XCTAssertFalse($0.rows.contains { $0.scope == "cloudOnly" }, "the Mac's own view has no such project") }
    }

    func testMacsThatCannotBeAskedAreListedWithTheirReason() async {
        await make(.offline)
        await composer.loadProjects()
        XCTAssertEqual(composer.sources.map(\.id), ["icloud", "mac:" + home, "mac:" + office])
        XCTAssertEqual(source(.mac(home))?.state, .failed(.offline))
        XCTAssertEqual(source(.mac(office))?.state, .failed(.offline))
        XCTAssertEqual(source(.icloud)?.choice.availability, .noSync, "nothing is known of any Mac syncing")
        XCTAssertTrue(composer.sources.allSatisfy { !$0.canOpen }, "there is nothing behind them")

        await make(.outdated)
        await composer.loadProjects()
        XCTAssertEqual(source(.mac(home))?.state, .failed(.needsUpdate))
        XCTAssertEqual(source(.mac(office))?.state, .failed(.needsUpdate))
        XCTAssertTrue(composer.sources.allSatisfy { !$0.canOpen })

        await make()
        await goOffline(office)
        await composer.loadProjects()
        XCTAssertEqual(source(.mac(office))?.state, .failed(.offline))
        XCTAssertFalse(rows(in: .icloud).isEmpty, "the Mac that is there is asked")
        XCTAssertEqual(composer.sources.map(\.canOpen), [true, false])
    }

    func testICloudWithNoMacThatSyncsOnlineSaysSo() async {
        await make()
        await composer.loadProjects()
        await goOffline(home)
        await composer.loadProjects()
        XCTAssertEqual(source(.icloud)?.state, .failed(.noSyncMac(anyOnline: true)), "the Office iMac is up, but it does not sync")
        XCTAssertFalse(source(.icloud)?.canOpen ?? true)
        XCTAssertEqual(source(.icloud)?.choice.members.map(\.online), [false], "the Home MacBook Pro is still what iCloud is made of")
        XCTAssertFalse(rows(in: .mac(office)).isEmpty)
    }

    func testASourceThatDoesNotAnswerIsMarkedSoAndCanBeAskedAgain() async throws {
        // The Todos tab has heard before that the Home MacBook Pro syncs and the Office iMac does not.
        let defaults = freshDefaults()
        defaults.set(try JSONEncoder().encode([home: "icloud", office: "local"]), forKey: "ah.todos.modes")
        await make(defaults: defaults)
        await goOffline(office)

        await backend.setLoseNextTodoProjectAnswers(1)
        await composer.loadProjects()
        XCTAssertEqual(source(.icloud)?.state, .failed(.timedOut))
        XCTAssertTrue(source(.icloud)?.canOpen ?? false, "it can be asked again")

        await composer.retry(.icloud)
        XCTAssertFalse(rows(in: .icloud).isEmpty)
    }

    func testAMacThatDoesNotAnswerBeforeAnythingIsKnownOfItIsAskedAgainOnItsOwn() async {
        await make()
        await goOffline(office)

        await backend.setLoseNextTodoProjectAnswers(1)
        await composer.loadProjects()
        XCTAssertEqual(source(.mac(home))?.state, .failed(.timedOut), "nothing says yet that it is part of iCloud")
        XCTAssertTrue(source(.mac(home))?.canOpen ?? false)

        await composer.retry(.mac(home))
        XCTAssertFalse(rows(in: .icloud).isEmpty, "it syncs: what it lists is iCloud's now")
        XCTAssertNil(source(.mac(home)), "and it has no source of its own")
    }

    func testTheSourcesWaitUntilTheMacsHaveSaidWhatTheyDoWithTheirTodos() async {
        await make()
        // Nothing is known yet of which Mac is part of iCloud, so the sources are not final.
        let loading = composer.startLoadingProjects()
        XCTAssertTrue(composer.isLearning)
        await loading.value
        XCTAssertFalse(composer.isLearning)
        XCTAssertEqual(composer.sources.map(\.id), ["icloud", "mac:" + office])
    }

    func testWhatTheTodosTabKnowsOfTheMacsMakesTheSourcesFinalAtOnce() async throws {
        let defaults = freshDefaults()
        defaults.set(try JSONEncoder().encode([home: "icloud", office: "local"]), forKey: "ah.todos.modes")
        await make(defaults: defaults)
        composer.startLoadingProjects()
        XCTAssertFalse(composer.isLearning)
        XCTAssertEqual(composer.sources.map(\.id), ["icloud", "mac:" + office])
        XCTAssertTrue(composer.sources.allSatisfy(\.canOpen), "its projects are on their way")
    }

    func testTheWaitEndsForAMacThatDoesNotAnswerAndForOneThatIsOffline() async {
        await make()
        await goOffline(office)
        await backend.setLoseNextTodoProjectAnswers(1)
        let loading = composer.startLoadingProjects()
        XCTAssertTrue(composer.isLearning, "the Home MacBook Pro is being asked")
        await loading.value
        XCTAssertFalse(composer.isLearning, "it did not answer, and that is the end of the wait; the Office iMac was never asked")
    }

    func testICloudIsServedByTheMacTheTodosTabUsedLast() async throws {
        await make(.bothSync)
        // The Home MacBook Pro is first in line, but the Office iMac served iCloud last.
        defaults.set(office, forKey: TodosStore.icloudMacKey)
        await composer.loadProjects()
        XCTAssertEqual(source(.icloud)?.servedBy, "Office iMac")
        let project = try XCTUnwrap(rows(in: .icloud).first { $0.repo == askHumanRepo })
        XCTAssertEqual(project.macId, office, "listed by the Mac that serves it")
        XCTAssertEqual(project.id, MockData.todoOfficeWorkProject)
    }

    func testTheSearchKeepsTheMatchingProjectsOfASource() async {
        await make()
        await composer.loadProjects()

        composer.query = "playground"
        XCTAssertEqual(composer.sections(of: .mac(office)).flatMap(\.rows).map(\.id), [MockData.todoOfficePlayProject])
        XCTAssertTrue(composer.sections(of: .icloud).isEmpty, "iCloud has no such project")

        composer.query = "dotfiles"
        XCTAssertEqual(composer.sections(of: .icloud).flatMap(\.rows).map(\.name), ["dotfiles"], "a search opens the projects without todos")

        composer.query = "no such project anywhere"
        XCTAssertTrue(composer.sections(of: .icloud).isEmpty)
        XCTAssertTrue(composer.sections(of: .mac(office)).isEmpty)

        composer.query = "  "
        XCTAssertFalse(composer.sections(of: .icloud).isEmpty, "a blank search is no search")
    }

    func testAListThatIsOpenFollowsAMacComingOnlineAndAClosedOneDoesNot() async {
        await make(.offline)
        await composer.loadProjects()
        XCTAssertEqual(source(.mac(home))?.state, .failed(.offline))

        await backend.setLink(.online, for: home)
        await eventually("the Mac to be listed") { !self.rows(in: .icloud).isEmpty }

        composer.closeProjects()
        await backend.setLink(.online, for: office)
        await eventually("the Mac to be seen") { self.app.state.macs.first { $0.id == self.office }?.link == .online }
        try? await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(source(.mac(office))?.state, .loading, "nobody is looking: nothing was asked")
    }
}
