import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

/// The Todos tab's state against the mock Macs: which source is shown, how the lists load, and what
/// happens to the user's writes before and after the Mac answers.
@MainActor
final class TodosStoreTests: XCTestCase {
    private var backend: MockBackend!
    private var app: AppStore!
    private var defaults: UserDefaults!
    private var todos: TodosStore { app.todos }

    private let home = MockData.homeMacId
    private let office = MockData.officeMacId

    private func make(_ scenario: MockBackend.Scenario = .demo) async {
        backend = MockBackend(scenario: scenario, latency: 0)
        defaults = UserDefaults(suiteName: "ah-todos-test-" + UUID().uuidString)!
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        await app.start()
        await eventually("first state") { self.app.state.foreground }
    }

    private func show() async {
        todos.setVisible(true)
        await eventually("the projects") { self.todos.projectsState == .loaded }
    }

    private func select(_ source: TodoSource) async {
        todos.select(source)
        await eventually("the projects of \(source)") { self.todos.source == source && self.todos.projectsState == .loaded }
    }

    private func watching(_ expected: Bool, _ message: String, mac: String? = nil) async {
        let id = mac ?? home
        for _ in 0..<300 {
            if await backend.isWatchingTodos(macId: id) == expected { return }
            try? await Task.sleep(for: .milliseconds(10))
        }
        XCTFail(message)
    }

    private func openAskHuman() async {
        guard let row = todos.sections.flatMap(\.rows).first(where: { $0.id == MockData.todoAskHumanProject }) else {
            return XCTFail("no AskHuman row")
        }
        todos.open(row)
        await eventually("the list") { self.todos.list != nil }
    }

    /// Both Macs sync: shown, with the projects only the Office iMac has asked for.
    private func showBothSyncing() async {
        await make(.bothSync)
        await show()
        await eventually("the other Mac's projects") { self.todos.others[self.office] != nil }
    }

    private func row(named name: String) -> TodoListedProject? {
        todos.sections.flatMap(\.rows).first { $0.name == name }
    }

    // MARK: sources

    func testItStartsOnTheFirstMacOnlineAndSettlesOnICloudOnceThatMacSaysItSyncs() async {
        await make()
        await show()
        XCTAssertEqual(todos.source, .icloud, "a Mac that syncs is part of iCloud, not a source of its own")
        XCTAssertEqual(todos.accessMacId, home)
        XCTAssertEqual(todos.modes[home], .icloud)
        await eventually("the other Mac's mode") { self.todos.modes[self.office] == .local }
        XCTAssertEqual(todos.choices.map(\.id), ["icloud", "mac:\(office)"])
        XCTAssertEqual(todos.choices[0].availability, .available, "a Mac that syncs makes iCloud choosable")
        XCTAssertEqual(todos.choices[0].members, [TodoSourceChoice.Member(name: "Home MacBook Pro", online: true)])
        XCTAssertFalse(todos.choices[1].syncs)
    }

    func testWhatTheMacsSaidIsKeptSoTheNextLaunchStartsOnICloud() async {
        await make()
        await show()
        let again = TodosStore(backend: backend, defaults: defaults)
        again.update(macs: app.state.macs, foreground: true)
        again.setVisible(true)
        XCTAssertEqual(again.source, .icloud, "no need to ask the Mac first")
        XCTAssertEqual(again.modes[home], .icloud)
    }

    func testAnEarlierChoiceOfAMacThatSyncsBecomesICloudAndOneOfAMacThatDoesNotStays() async {
        await make()
        await show()
        func launch(_ stored: TodoSource) -> TodosStore {
            defaults.set(stored.stored, forKey: "ah.todos.source")
            let store = TodosStore(backend: backend, defaults: defaults)
            store.update(macs: app.state.macs, foreground: true)
            store.setVisible(true)
            return store
        }
        XCTAssertEqual(launch(.mac(home)).source, .icloud)
        XCTAssertEqual(launch(.mac(office)).source, .mac(office), "the user's own choice is not touched")
    }

    func testAMacBehindICloudCannotBeSelectedOnItsOwn() async {
        await make()
        await show()
        todos.select(.mac(home))
        XCTAssertEqual(todos.source, .icloud)
        XCTAssertEqual(todos.projectsState, .loaded)
    }

    func testMacsThatPredateTodosAreNotAskedAndSayTheyNeedAnUpdate() async {
        await make(.outdated)
        todos.setVisible(true)
        await eventually("the verdict") { self.todos.projectsState == .failed(.needsUpdate) }
        XCTAssertEqual(todos.source, .mac(home), "the first Mac is shown even if it cannot be")
        XCTAssertNil(todos.projects)
        XCTAssertNil(todos.accessMacId, "nothing is subscribed to")
        XCTAssertEqual(todos.choices.map(\.availability), [.noSync, .needsUpdate, .needsUpdate])
    }

    func testAnOutdatedMacIsExplainedEvenWhenTheTabIsOnScreenBeforeTheMacsAreKnown() async {
        backend = MockBackend(scenario: .outdated, latency: 0)
        defaults = UserDefaults(suiteName: "ah-todos-test-" + UUID().uuidString)!
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        todos.setVisible(true)
        XCTAssertEqual(todos.projectsState, .failed(.noMacs), "nothing is paired as far as the store knows")
        await app.start()
        await eventually("the verdict") { self.todos.projectsState == .failed(.needsUpdate) }
        XCTAssertEqual(todos.source, .mac(home))
    }

    func testICloudShowsWhatSyncsAndAMacShowsWhatIsOnIt() async {
        await make()
        await show()
        await select(.icloud)
        XCTAssertEqual(todos.accessMacId, home)
        XCTAssertEqual(todos.accessName, "Home MacBook Pro")
        XCTAssertEqual(todos.sections.map(\.kind), [.synced, .empty, .localOnly])
        XCTAssertEqual(todos.sections[0].rows.map(\.name), ["AskHuman", "Billing Service", "blog", "notes"])
        XCTAssertTrue(todos.sections[1].collapsed)
        XCTAssertNotNil(todos.syncLine)

        await select(.mac(office))
        XCTAssertEqual(todos.sections.map(\.kind), [.withTodos, .recent])
        XCTAssertEqual(todos.sections[0].rows.compactMap(\.hint), ["work", "playground"], "same names are told apart")
        XCTAssertNil(todos.syncLine, "this Mac does not sync")
    }

    func testAMacThatSyncsAloneIsAlsoASourceOfItsOwnThatLeavesOutWhatOnlyICloudHas() async {
        await make()
        await backend.unpair(macId: office)
        await eventually("one Mac is left") { self.app.state.macs.count == 1 }
        await show()
        XCTAssertEqual(todos.source, .mac(home), "with one Mac there is nothing to merge")
        XCTAssertEqual(todos.choices.map(\.id), ["icloud", "mac:\(home)"])
        XCTAssertTrue(todos.choices[1].syncs)
        XCTAssertTrue(todos.choices[0].members.isEmpty)
        XCTAssertEqual(todos.sections[0].rows.map(\.name), ["AskHuman", "Billing Service", "blog"], "no directory here for a cloud-only project")

        await select(.icloud)
        XCTAssertEqual(todos.sections[0].rows.map(\.name), ["AskHuman", "Billing Service", "blog", "notes"])
    }

    // MARK: several Macs behind iCloud

    func testSeveralMacsThatSyncAreOneViewThatListsWhatEachHasAlone() async {
        await showBothSyncing()
        XCTAssertEqual(todos.source, .icloud)
        XCTAssertEqual(todos.choices.map(\.id), ["icloud"], "the Macs are what iCloud is made of")
        XCTAssertEqual(todos.choices[0].members.map(\.name), ["Home MacBook Pro", "Office iMac"])
        XCTAssertEqual(todos.accessMacId, home)

        XCTAssertEqual(todos.sections.map(\.kind), [.synced, .empty, .localOnly, .localOnly])
        XCTAssertEqual(todos.sections.map(\.macId), [nil, nil, home, office])
        let alone = todos.sections[3]
        XCTAssertEqual(alone.rows.map(\.name), ["intranet"])
        XCTAssertEqual(alone.rows.first?.macId, office)
        XCTAssertEqual(alone.rows.first?.macName, "Office iMac")
        XCTAssertEqual(todos.sections[1].total, 2, "the projects without todos of both Macs")

        todos.emptyExpanded = true
        let names = todos.sections.flatMap(\.rows).map(\.name)
        XCTAssertEqual(names.filter { $0 == "AskHuman" }.count, 1, "a synced project is told once, though both Macs have it")
        XCTAssertTrue(names.contains("sandbox"), "a project of the Office iMac that nothing else has")
        XCTAssertTrue(names.contains("scratch"))
    }

    func testAProjectOnlyTheOtherMacHasIsOpenedAndWrittenOnThatMac() async {
        await showBothSyncing()
        guard let intranet = row(named: "intranet") else { return XCTFail("no intranet row") }
        todos.open(intranet)
        await eventually("the list") { self.todos.list != nil }
        XCTAssertEqual(todos.opened?.macId, office)
        XCTAssertEqual(todos.openedMacName, "Office iMac")
        XCTAssertEqual(todos.list?.pending.map(\.text), ["Move the wiki off the old server"])
        XCTAssertTrue(todos.canWrite)
        await watching(true, "the Mac that has the project is watched", mac: office)
        await watching(true, "and so is the one serving the list", mac: home)

        XCTAssertNil(todos.add("Ask the admins", auto: false))
        await eventually("answered") { self.todos.writing == 0 }
        XCTAssertEqual(todos.list?.pending.map(\.text), ["Move the wiki off the old server", "Ask the admins"])
        XCTAssertEqual(row(named: "intranet")?.pending, 2, "the count in the list follows")

        await backend.simulateTodo(macId: office, projectId: MockData.todoOfficeIntranetProject, text: "From the office")
        await eventually("a change made there") { self.todos.list?.pending.contains { $0.text == "From the office" } ?? false }

        todos.close()
        await watching(false, "closing the project lets go of the other Mac", mac: office)
        await watching(true, "the list is still shown", mac: home)
    }

    func testAProjectThatMovesBetweenSectionsWithItsWriteDoesSoOnTheOtherMacsRow() async {
        await showBothSyncing()
        todos.emptyExpanded = true
        guard let sandbox = row(named: "sandbox") else { return XCTFail("no sandbox row") }
        XCTAssertEqual(todos.sections.first { $0.rows.contains { $0.key == sandbox.key } }?.kind, .empty)
        todos.open(sandbox)
        await eventually("the list") { self.todos.list != nil }
        XCTAssertNil(todos.add("Reset the sandbox", auto: false))
        await eventually("answered") { self.todos.writing == 0 }
        let moved = todos.sections.first { $0.rows.contains { $0.key == sandbox.key } }
        XCTAssertEqual(moved?.kind, .localOnly)
        XCTAssertEqual(moved?.macId, office)
    }

    func testWhenTheMacServingICloudGoesAwayTheOtherOneTakesOverAndWhatOnlyTheFirstHadStaysListed() async {
        await showBothSyncing()
        await backend.setLink(.macOffline, for: home)
        await eventually("the other Mac serves") { self.todos.accessMacId == self.office && self.todos.projectsState == .loaded }
        XCTAssertEqual(todos.source, .icloud)
        XCTAssertEqual(todos.choices[0].members.map(\.online), [false, true])
        XCTAssertEqual(todos.accessName, "Office iMac")

        guard let scratch = row(named: "scratch") else { return XCTFail("the first Mac's own project is gone") }
        XCTAssertEqual(scratch.macId, home)
        todos.open(scratch)
        XCTAssertEqual(todos.openedMacName, "Home MacBook Pro", "the page names the Mac that is not there")
        await eventually("the list says why") { self.todos.listState == .failed(.offline) }
        XCTAssertFalse(todos.canWrite)
        XCTAssertNil(todos.list, "there is nothing to show of it until the Mac is back")
    }

    func testAMacThatStopsSyncingComesBackInTheMenuAndItsOwnProjectsLeaveICloud() async {
        await showBothSyncing()
        await backend.setTodoSyncing([home])
        await todos.refresh()
        XCTAssertEqual(todos.modes[office], .local)
        XCTAssertNil(todos.others[office])
        XCTAssertEqual(todos.choices.map(\.id), ["icloud", "mac:\(office)"])
        XCTAssertEqual(todos.sections.map(\.kind), [.synced, .empty, .localOnly])
        XCTAssertEqual(todos.source, .icloud)
    }

    func testPullingDownLearnsThatAMacStartedToSync() async {
        await make()
        await show()
        await eventually("the other Mac's mode") { self.todos.modes[self.office] == .local }
        XCTAssertEqual(todos.choices.map(\.id), ["icloud", "mac:\(office)"])

        await backend.setTodoSyncing([home, office])
        await todos.refresh()
        XCTAssertEqual(todos.modes[office], .icloud)
        XCTAssertNotNil(todos.others[office])
        XCTAssertEqual(todos.choices.map(\.id), ["icloud"])
    }

    // MARK: the dots

    func testTheAgentsOfEveryMacMarkTheProjectsTheyWorkIn() async {
        await showBothSyncing()
        await eventually("the Agents") { !self.app.state.agents.isEmpty }
        XCTAssertEqual(row(named: "AskHuman").flatMap(todos.activity(of:)), .waiting, "Home's Agent that asks wins over the one that works")
        XCTAssertEqual(
            row(named: "Billing Service").flatMap(todos.activity(of:)), .working,
            "the Agent is on the Office iMac, which does not serve the list"
        )
        XCTAssertNil(row(named: "blog").flatMap(todos.activity(of:)), "no Agent there")
    }

    func testADotFollowsTheAgentsStateAndGoesWhenTheSessionEnds() async {
        await showBothSyncing()
        await eventually("the Agents") { !self.app.state.agents.isEmpty }
        func billing() -> TodoActivity? { row(named: "Billing Service").flatMap(todos.activity(of:)) }
        XCTAssertEqual(billing(), .working)

        await backend.setAgentState(macId: office, agentKey: MockData.officeAgentKey, to: "waiting")
        await eventually("waiting") { billing() == .waiting }

        await backend.setAgentState(macId: office, agentKey: MockData.officeAgentKey, to: "ended")
        await eventually("only the idle one is left") { billing() == .idle }

        await backend.setAgentState(macId: office, agentKey: MockData.unsupportedAgentKey, to: "ended")
        await eventually("no dot") { billing() == nil }
    }

    func testAnEarlierSourceComesBackAtOnce() async {
        await make()
        await show()
        await select(.icloud)
        await select(.mac(office))
        todos.select(.icloud)
        XCTAssertEqual(todos.projectsState, .loaded, "the rows kept from before are shown while they load again")
        XCTAssertFalse(todos.sections.isEmpty)
    }

    func testTheChoiceIsRemembered() async {
        await make()
        await show()
        await select(.mac(office))
        let again = TodosStore(backend: backend, defaults: defaults)
        again.update(macs: app.state.macs, foreground: true)
        again.setVisible(true)
        XCTAssertEqual(again.source, .mac(office))
    }

    func testICloudWithoutAMacThatSyncsSaysWhy() async {
        await make()
        await backend.setTodoSyncing([])
        await show()
        todos.select(.icloud)
        await eventually("the failure") { self.todos.projectsState == .failed(.noSyncMac(anyOnline: true)) }
        XCTAssertNil(todos.accessMacId)
        XCTAssertTrue(todos.sections.isEmpty)
    }

    func testICloudMovesToAnotherMacWhenTheOneServingItGoesAway() async {
        await make()
        await backend.setTodoSyncing([home, office])
        await show()
        await select(.icloud)
        XCTAssertEqual(todos.accessMacId, home)
        await openAskHuman()

        await backend.setLink(.macOffline, for: home)
        await eventually("another Mac to take over") { self.todos.accessMacId == self.office && self.todos.projectsState == .loaded }
        XCTAssertNil(todos.opened, "a project of the Mac that went away cannot stay open")
    }

    func testICloudKeepsItsRowsWhenTheMacDoesNotAnswer() async {
        await make()
        await show()
        await select(.icloud)
        let rows = todos.sections.flatMap(\.rows).map(\.id)
        XCTAssertFalse(rows.isEmpty)
        await openAskHuman()

        // Still online as far as the phone knows, but the answer does not come.
        await backend.setUnreachable([home])
        await todos.refresh()
        XCTAssertEqual(todos.projectsState, .failed(.offline), "the page's note has its reason")
        XCTAssertEqual(todos.sections.flatMap(\.rows).map(\.id), rows, "what the Mac last said stays")
        XCTAssertEqual(todos.accessMacId, home)
        XCTAssertNotNil(todos.syncLine)
        XCTAssertNotNil(todos.opened, "the project that is open stays open")

        await backend.setUnreachable([])
        await todos.refresh()
        XCTAssertEqual(todos.projectsState, .loaded)
        XCTAssertEqual(todos.sections.flatMap(\.rows).map(\.id), rows)
    }

    func testICloudWithNothingToShowSaysTheMacDidNotAnswerNotThatSyncIsOff() async {
        await make()
        await backend.setUnreachable([home])
        todos.setVisible(true)
        await eventually("the Mac that was chosen first does not answer") { self.todos.projectsState == .failed(.offline) }

        todos.select(.icloud)
        await eventually("iCloud's failure") { self.todos.source == .icloud && self.todos.projectsState.isFailed }
        XCTAssertEqual(todos.projectsState, .failed(.offline), "a Mac was asked and said nothing; that is not \"no Mac syncs\"")
        XCTAssertNil(todos.projects)
        XCTAssertNil(todos.accessMacId)
    }

    func testICloudKeepsItsRowsWhenTheMacServingItGoesAway() async {
        await make()
        await show()
        await select(.icloud)
        let rows = todos.sections.flatMap(\.rows).map(\.id)
        XCTAssertFalse(rows.isEmpty)
        await openAskHuman()
        XCTAssertTrue(todos.canWrite)

        // The Macs' own presence says the one serving iCloud is gone and the other does not sync: nobody can take over.
        await backend.setLink(.macOffline, for: home)
        await eventually("the page's note") { self.todos.projectsState == .failed(.offline) }
        XCTAssertEqual(todos.sections.flatMap(\.rows).map(\.id), rows, "what the Mac last said stays")
        XCTAssertEqual(todos.accessMacId, home, "the rows still belong to it")
        XCTAssertNotNil(todos.syncLine)
        XCTAssertNotNil(todos.opened, "the project that is open stays open")
        XCTAssertNotNil(todos.list)
        XCTAssertFalse(todos.canWrite, "read-only until it is back")

        await backend.setLink(.online, for: home)
        await eventually("loaded again") { self.todos.projectsState == .loaded }
        XCTAssertTrue(todos.canWrite)
        XCTAssertEqual(todos.sections.flatMap(\.rows).map(\.id), rows)
    }

    func testICloudComesBackFromTheCacheWithItsNoteWhenItsMacHasGone() async {
        await make()
        await show()
        await select(.icloud)
        let rows = todos.sections.flatMap(\.rows).map(\.id)
        await select(.mac(office))
        await backend.setLink(.macOffline, for: home)

        todos.select(.icloud)
        await eventually("the page's note") { self.todos.source == .icloud && self.todos.projectsState == .failed(.offline) }
        XCTAssertEqual(todos.sections.flatMap(\.rows).map(\.id), rows, "the rows kept from before are not thrown away")
        XCTAssertEqual(todos.accessMacId, home)
    }

    func testICloudStillEmptiesWhenNoMacSyncsAnyMore() async {
        await make()
        await show()
        await select(.icloud)
        XCTAssertFalse(todos.sections.isEmpty)

        // The Mac is there and says it does not sync: its rows are not iCloud's view of the todos any more.
        await backend.setTodoSyncing([])
        await todos.refresh()
        XCTAssertEqual(todos.projectsState, .failed(.noSyncMac(anyOnline: true)))
        XCTAssertNil(todos.projects)
        XCTAssertTrue(todos.sections.isEmpty)
        XCTAssertNil(todos.accessMacId)
    }

    func testICloudEmptiesWhenTheMacServingItIsUnpaired() async {
        await make()
        await show()
        await select(.icloud)
        XCTAssertEqual(todos.accessMacId, home)

        await backend.unpair(macId: home)
        await eventually("nothing of it is left") { self.todos.projects == nil }
        XCTAssertTrue(todos.sections.isEmpty)
        XCTAssertNil(todos.accessMacId)
        XCTAssertTrue(todos.projectsState.isFailed)
    }

    func testAMacThatGoesAwayLeavesItsRowsOnScreenWithTheFailure() async {
        await make()
        await show()
        await backend.setLink(.macOffline, for: home)
        await eventually("the failure") { self.todos.projectsState == .failed(.offline) }
        XCTAssertFalse(todos.sections.isEmpty, "what was shown stays")
        XCTAssertFalse(todos.canWrite)

        await backend.setLink(.online, for: home)
        await eventually("loaded again") { self.todos.projectsState == .loaded }
        XCTAssertTrue(todos.canWrite)
    }

    func testEveryMacUnpairedLeavesNothingToShow() async {
        await make()
        await show()
        await backend.forgetAll()
        await eventually("no source") { self.todos.source == nil }
        XCTAssertEqual(todos.projectsState, .failed(.noMacs))
    }

    func testRefreshesThatOverlapAreMergedNotRefused() async {
        await make()
        await show()
        async let first: Void = todos.refresh()
        async let second: Void = todos.refresh()
        _ = await (first, second)
        XCTAssertEqual(todos.projectsState, .loaded)
    }

    // MARK: writes

    func testAnAddedTodoShowsAtOnceAndTheMacConfirmsIt() async {
        await make()
        await show()
        await openAskHuman()
        let before = todos.list?.pending.count ?? 0
        let rowBefore = todos.projects?.projects.first { $0.id == MockData.todoAskHumanProject }?.pending

        XCTAssertNil(todos.add("  Ship it  ", auto: false))
        XCTAssertEqual(todos.list?.pending.count, before + 1, "shown before the Mac has answered")
        XCTAssertEqual(todos.list?.pending.last?.text, "Ship it")
        let local = todos.list?.pending.last?.id ?? ""
        XCTAssertTrue(todos.isSending(local))
        XCTAssertEqual(todos.writing, 1)

        await eventually("the Mac's answer") { self.todos.writing == 0 }
        XCTAssertEqual(todos.list?.pending.count, before + 1)
        XCTAssertEqual(todos.list?.pending.last?.text, "Ship it")
        XCTAssertFalse(todos.list?.pending.last?.id.hasPrefix("local-") ?? true, "the Mac's id replaced the local one")
        XCTAssertFalse(todos.isSending(local))
        XCTAssertEqual(todos.projects?.projects.first { $0.id == MockData.todoAskHumanProject }?.pending, (rowBefore ?? 0) + 1, "the row count follows")
        let ops = await backend.todoOps
        XCTAssertEqual(ops.map(\.op), ["add"])
        XCTAssertEqual(ops.first?.text, "Ship it")
    }

    func testTextThatCannotBeStoredNeverGoesOut() async {
        await make()
        await show()
        await openAskHuman()
        XCTAssertEqual(todos.add("   \n ", auto: false), .empty)
        XCTAssertEqual(todos.add(String(repeating: "x", count: Wire.todoTextMaxChars + 1), auto: false), .tooLong)
        XCTAssertEqual(todos.edit("todo-ah-1", text: ""), .empty)
        XCTAssertEqual(todos.writing, 0)
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty)
    }

    func testCompletingAndRestoringMoveATodoBetweenTheQueueAndTheHistory() async {
        await make()
        await show()
        await openAskHuman()
        let pending = todos.list?.pending.count ?? 0
        let history = todos.list?.history.count ?? 0

        todos.complete("todo-ah-2")
        XCTAssertEqual(todos.list?.pending.count, pending - 1, "gone from the queue at once")
        XCTAssertEqual(todos.list?.history.first?.id, "todo-ah-2")
        await eventually("answered") { self.todos.writing == 0 }
        XCTAssertEqual(todos.list?.history.count, history + 1)

        todos.restore("todo-ah-2")
        await eventually("answered again") { self.todos.writing == 0 }
        XCTAssertEqual(todos.list?.pending.last?.id, "todo-ah-2", "restored to the end of the queue")
        XCTAssertEqual(todos.list?.history.count, history)
    }

    func testSeveralWritesGoOutOneAfterAnotherInOrder() async {
        await make()
        await show()
        await openAskHuman()
        todos.complete("todo-ah-1")
        todos.edit("todo-ah-2", text: "Renamed")
        todos.setAuto("todo-ah-3", true)
        todos.move(from: IndexSet(integer: 0), to: 3)
        XCTAssertEqual(todos.writing, 4)
        await eventually("all answered") { self.todos.writing == 0 }
        let ops = await backend.todoOps
        XCTAssertEqual(ops.map(\.op), ["complete", "edit", "auto", "reorder"])
        let list = todos.list
        XCTAssertEqual(list?.pending.map(\.id), ["todo-ah-3", "todo-ah-4", "todo-ah-2"], "the Mac's order is the final one")
        XCTAssertEqual(list?.pending.first { $0.id == "todo-ah-2" }?.text, "Renamed")
        XCTAssertEqual(list?.pending.first { $0.id == "todo-ah-3" }?.auto, true)
    }

    func testAnAnswerThatArrivesDuringAWriteDoesNotBringBackTheOldList() async {
        await make()
        await show()
        await openAskHuman()
        todos.complete("todo-ah-1")
        todos.reloadList()
        todos.changed(macId: home, rev: 999)
        XCTAssertFalse(todos.list?.pending.contains { $0.id == "todo-ah-1" } ?? true)
        await eventually("settled") { self.todos.writing == 0 }
        await eventually("quiet") { self.todos.listState == .loaded }
        XCTAssertFalse(todos.list?.pending.contains { $0.id == "todo-ah-1" } ?? true)
    }

    func testARefusedWriteIsTakenBackWithANotice() async {
        await make()
        await show()
        await openAskHuman()
        await backend.setFailNextTodoWrite("disk")
        todos.complete("todo-ah-1")
        await eventually("answered") { self.todos.writing == 0 }
        XCTAssertTrue(todos.list?.pending.contains { $0.id == "todo-ah-1" } ?? false, "the todo is back")
        XCTAssertEqual(todos.notice?.notice, .notSaved)
    }

    func testATodoDoneElsewhereIsSaidSoAndTheListCatchesUp() async {
        await make()
        await show()
        await openAskHuman()
        todos.complete("todo-ah-1")
        await eventually("first answered") { self.todos.writing == 0 }
        todos.complete("todo-ah-1")
        await eventually("second answered") { self.todos.writing == 0 }
        XCTAssertEqual(todos.notice?.notice, .changedElsewhere)
        XCTAssertEqual(todos.list?.history.filter { $0.id == "todo-ah-1" }.count, 1)
    }

    func testAMacThatCannotBeReachedTakesTheWriteBackAndSaysSo() async {
        await make()
        await show()
        await openAskHuman()
        let before = todos.list?.pending.map(\.id)
        await backend.setUnreachable([home])
        todos.complete("todo-ah-1")
        await eventually("failed") { self.todos.writing == 0 && self.todos.notice != nil }
        XCTAssertEqual(todos.notice?.notice, .offline)
        XCTAssertEqual(todos.list?.pending.map(\.id), before, "nothing was kept for later")
    }

    func testNothingIsQueuedForAMacThatIsKnownToBeOffline() async {
        await make()
        await show()
        await openAskHuman()
        await backend.setLink(.macOffline, for: home)
        await eventually("offline") { !self.todos.canWrite }
        todos.complete("todo-ah-1")
        XCTAssertEqual(todos.writing, 0)
        XCTAssertEqual(todos.notice?.notice, .offline)
        let ops = await backend.todoOps
        XCTAssertTrue(ops.isEmpty)
    }

    func testAWriteToACloudOnlyProjectCannotSetAutoRun() async {
        await make()
        await show()
        await select(.icloud)
        guard let row = todos.sections.flatMap(\.rows).first(where: { $0.id == MockData.todoNotesProject }) else {
            return XCTFail("no notes row")
        }
        todos.open(row)
        await eventually("the list") { self.todos.list != nil }
        XCTAssertEqual(todos.list?.canAuto, false)
        XCTAssertNil(todos.add("A note", auto: true))
        await eventually("answered") { self.todos.writing == 0 }
        XCTAssertEqual(todos.notice?.notice, .refused("cloudOnly"))
        XCTAssertFalse(todos.list?.pending.contains { $0.text == "A note" } ?? true)
    }

    // MARK: freshness

    func testAChangeMadeElsewhereReachesTheOpenProject() async {
        await make()
        await show()
        await openAskHuman()
        await watching(true, "the screen on show subscribes")
        await backend.simulateTodo(macId: home, projectId: MockData.todoAskHumanProject, text: "From elsewhere")
        await eventually("the new todo") { self.todos.list?.pending.contains { $0.text == "From elsewhere" } ?? false }
        await eventually("the project row follows") {
            self.todos.projects?.projects.first { $0.id == MockData.todoAskHumanProject }?.pending == self.todos.list?.pendingTotal
        }
    }

    func testHidingTheTabOrBackgroundingTheAppStopsTheSubscription() async {
        await make()
        await show()
        await watching(true, "subscribed while shown")

        todos.setVisible(false)
        await watching(false, "a hidden tab asks for nothing")

        todos.setVisible(true)
        await watching(true, "subscribed again")

        await app.setForeground(false)
        await watching(false, "a backgrounded app asks for nothing")
    }

    func testTheSyncLineFollowsTheMacsStatus() async {
        await make()
        await backend.setTodoSyncProblem("notSignedIn")
        await show()
        XCTAssertEqual(todos.syncLine, .notSignedIn)
    }

    // MARK: the sync line while a pull runs

    /// A store that looks again fast enough for a test: `delays` are the waits between the looks.
    private func quickStore(_ delays: [Duration] = Array(repeating: .milliseconds(20), count: 40)) -> TodosStore {
        let store = TodosStore(backend: backend, defaults: defaults, syncRecheckDelays: delays)
        store.update(macs: app.state.macs, foreground: true)
        return store
    }

    private func isSynced(_ line: TodoSyncLine?) -> Bool {
        if case .synced? = line { return true }
        return false
    }

    func testTheSyncLineStopsSayingSyncingOnceTheMacIsDone() async {
        await make()
        await backend.setTodoSyncRunning(true)
        let store = quickStore()
        store.setVisible(true)
        await eventually("the first answer") { store.projectsState == .loaded }
        XCTAssertEqual(store.syncLine, .syncing, "the Mac was in the middle of its pull when it answered")

        await backend.setTodoSyncRunning(false)
        await eventually("the line catches up without being asked to") { self.isSynced(store.syncLine) }
        XCTAssertEqual(store.rechecks, 0, "nothing is left to look at")
    }

    func testItGivesUpAfterItsLooksAndPullingDownAsksAgain() async {
        await make()
        await backend.setTodoSyncRunning(true)
        let store = quickStore([.milliseconds(10), .milliseconds(10), .milliseconds(10)])
        store.setVisible(true)
        await eventually("every look is used") { store.rechecks == 3 }
        try? await Task.sleep(for: .milliseconds(150))
        XCTAssertEqual(store.rechecks, 3, "no fourth look")
        XCTAssertEqual(store.syncLine, .syncing, "the line says what the Mac last said")

        await backend.setTodoSyncRunning(false)
        await store.refresh()
        XCTAssertTrue(isSynced(store.syncLine))
    }

    func testALookThatGetsNoAnswerLeavesTheScreenAlone() async {
        await make()
        await backend.setTodoSyncRunning(true)
        let store = quickStore(Array(repeating: .milliseconds(40), count: 30))
        store.setVisible(true)
        await eventually("the first answer") { store.projectsState == .loaded }
        store.select(.icloud)
        await eventually("iCloud") { store.source == .icloud && store.projectsState == .loaded && store.accessMacId == self.home }

        await backend.setUnreachable([home])
        let looked = store.rechecks
        await eventually("the looks go on, each one lost") { store.rechecks >= looked + 3 }
        XCTAssertEqual(store.projectsState, .loaded, "a lost look is not a failure")
        XCTAssertFalse(store.sections.isEmpty, "the rows stay")
        XCTAssertEqual(store.accessMacId, home)
        XCTAssertEqual(store.syncLine, .syncing, "and so does the line")
    }

    func testLeavingTheTabOrTheAppStopsTheLooks() async {
        await make()
        await backend.setTodoSyncRunning(true)
        let store = quickStore(Array(repeating: .milliseconds(30), count: 40))
        store.setVisible(true)
        await eventually("the first look") { store.rechecks >= 1 }

        store.setVisible(false)
        XCTAssertEqual(store.rechecks, 0)
        try? await Task.sleep(for: .milliseconds(150))
        XCTAssertEqual(store.rechecks, 0, "a hidden tab asks for nothing")

        store.setVisible(true)
        await eventually("looking again") { store.rechecks >= 1 }
        store.update(macs: app.state.macs, foreground: false)
        XCTAssertEqual(store.rechecks, 0)
        try? await Task.sleep(for: .milliseconds(150))
        XCTAssertEqual(store.rechecks, 0, "a backgrounded app asks for nothing")
    }

    func testTheLooksAreFewAndTheyEndWithinHalfAMinuteOrSo() {
        let delays = TodosStore.syncRecheckDelays
        XCTAssertLessThanOrEqual(delays.first ?? .seconds(99), .seconds(2), "the first look comes soon")
        let total = delays.reduce(Duration.zero, +)
        XCTAssertGreaterThanOrEqual(total, .seconds(30))
        XCTAssertLessThanOrEqual(total, .seconds(40))
    }
}
