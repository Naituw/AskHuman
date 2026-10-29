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

    private func watching(_ expected: Bool, _ message: String) async {
        for _ in 0..<300 {
            if await backend.isWatchingTodos(macId: home) == expected { return }
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

    // MARK: sources

    func testItStartsOnTheFirstMacOnlineAndLearnsWhatTheOthersDo() async {
        await make()
        await show()
        XCTAssertEqual(todos.source, .mac(home))
        XCTAssertEqual(todos.accessMacId, home)
        XCTAssertEqual(todos.modes[home], .icloud)
        await eventually("the other Mac's mode") { self.todos.modes[self.office] == .local }
        XCTAssertEqual(todos.choices.map(\.id), ["icloud", "mac:\(home)", "mac:\(office)"])
        XCTAssertEqual(todos.choices[0].availability, .available, "a Mac that syncs makes iCloud choosable")
        XCTAssertTrue(todos.choices[1].syncs)
        XCTAssertFalse(todos.choices[2].syncs)
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

        await select(.mac(home))
        XCTAssertEqual(todos.sections[0].rows.map(\.name), ["AskHuman", "Billing Service", "blog"], "no directory here for a cloud-only project")

        await select(.mac(office))
        XCTAssertEqual(todos.sections.map(\.kind), [.withTodos, .recent])
        XCTAssertEqual(todos.sections[0].rows.compactMap(\.hint), ["work", "playground"], "same names are told apart")
        XCTAssertNil(todos.syncLine, "this Mac does not sync")
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

    func testICloudStillEmptiesWhenNoMacIsThereToServeIt() async {
        await make()
        await show()
        await select(.icloud)
        XCTAssertFalse(todos.sections.isEmpty)

        // The Macs' own presence says the one serving iCloud is gone and the other does not sync: nothing to ask.
        await backend.setLink(.macOffline, for: home)
        await eventually("iCloud has nobody") { self.todos.projectsState == .failed(.noSyncMac(anyOnline: true)) }
        XCTAssertNil(todos.projects)
        XCTAssertTrue(todos.sections.isEmpty)
        XCTAssertNil(todos.accessMacId)
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
