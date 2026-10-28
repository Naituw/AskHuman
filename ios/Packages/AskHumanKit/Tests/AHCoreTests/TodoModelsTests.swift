import XCTest
@testable import AHProtocol
@testable import AHCore

/// The pure rules behind the todo screens: sources, grouping, search, the sync line and the changes a
/// write makes before the Mac has answered.
final class TodoModelsTests: XCTestCase {
    private func mac(_ id: String, link: MacLink = .online, todos: Bool = true) -> MacState {
        MacState(
            id: id, name: "Mac " + id, macName: "Mac " + id, alias: nil, link: link,
            caps: Caps(newTask: true, interject: true, watch: true, todos: todos), slot: nil, pairedAt: Date(timeIntervalSince1970: 0),
            lastSeenAt: nil, pendingCount: 0, pushStatus: nil
        )
    }

    private func row(_ id: String, name: String = "P", section: String = "withTodos", scope: String = "local", hint: String? = nil, repo: String? = nil, macs: [String] = []) -> TodoProjectRow {
        TodoProjectRow(id: id, name: name, hint: hint, repo: repo, scope: scope, section: section, macs: macs)
    }

    // MARK: sources

    func testASourceSurvivesBeingStored() {
        XCTAssertEqual(TodoSource(stored: TodoSource.icloud.stored), .icloud)
        XCTAssertEqual(TodoSource(stored: TodoSource.mac("abc").stored), .mac("abc"))
        XCTAssertNil(TodoSource(stored: "mac:"))
        XCTAssertNil(TodoSource(stored: "elsewhere"))
    }

    func testTheMenuNamesWhatEachSourceCanDo() {
        let macs = [mac("a"), mac("b", link: .macOffline), mac("c", todos: false)]
        var choices = TodoSources.choices(macs: macs, modes: [:])
        XCTAssertEqual(choices.map(\.id), ["icloud", "mac:a", "mac:b", "mac:c"])
        XCTAssertEqual(choices[0].availability, .noSync, "no Mac is known to sync")
        XCTAssertEqual(choices[1].availability, .available)
        XCTAssertEqual(choices[2].availability, .offline)
        XCTAssertEqual(choices[3].availability, .needsUpdate)

        choices = TodoSources.choices(macs: macs, modes: ["a": .icloud, "b": .local])
        XCTAssertEqual(choices[0].availability, .available)
        XCTAssertTrue(choices[1].syncs)
        XCTAssertFalse(choices[2].syncs)
    }

    func testTheFirstSourceIsTheLastChoiceThenTheOnlyMacThenTheFirstOnline() {
        XCTAssertNil(TodoSources.initial(stored: nil, macs: [], modes: [:]))
        XCTAssertEqual(TodoSources.initial(stored: nil, macs: [mac("a", link: .macOffline)], modes: [:]), .mac("a"), "the only Mac")
        let two = [mac("a", link: .macOffline), mac("b")]
        XCTAssertEqual(TodoSources.initial(stored: nil, macs: two, modes: [:]), .mac("b"), "the first one that is online")
        XCTAssertEqual(TodoSources.initial(stored: .mac("a"), macs: two, modes: [:]), .mac("a"), "what the user chose last")
        XCTAssertEqual(TodoSources.initial(stored: .mac("gone"), macs: two, modes: [:]), .mac("b"), "a Mac that was unpaired")
        XCTAssertEqual(TodoSources.initial(stored: .icloud, macs: two, modes: [:]), .icloud, "kept until a Mac says it does not sync")
        XCTAssertEqual(TodoSources.initial(stored: .icloud, macs: two, modes: ["a": .local, "b": .local]), .mac("b"))
        XCTAssertEqual(TodoSources.initial(stored: .icloud, macs: [mac("a", todos: false)], modes: [:]), .mac("a"))
    }

    func testICloudIsServedByTheBestMacOnline() {
        let macs = [mac("a"), mac("b"), mac("c"), mac("d", link: .macOffline), mac("e", todos: false)]
        // Known to sync first, then unknown ones; offline, old and non-syncing Macs never.
        XCTAssertEqual(
            TodoSources.icloudCandidates(macs: macs, modes: ["b": .icloud, "c": .local], lastUsed: nil), ["b", "a"]
        )
        // The Mac that served it last time stays first.
        XCTAssertEqual(
            TodoSources.icloudCandidates(macs: macs, modes: ["a": .icloud, "b": .icloud], lastUsed: "b"), ["b", "a", "c"]
        )
        // One that failed this time is skipped.
        XCTAssertEqual(
            TodoSources.icloudCandidates(macs: macs, modes: ["a": .icloud, "b": .icloud], lastUsed: "b", failed: ["b"]), ["a", "c"]
        )
        XCTAssertEqual(TodoSources.icloudCandidates(macs: [mac("d", link: .macOffline)], modes: [:], lastUsed: nil), [])
    }

    // MARK: rows

    func testRowsAreGroupedAsTheDesktopDoesAndAMacsOwnViewLeavesOutCloudOnlyProjects() {
        let rows = [
            row("1", name: "Synced", section: "synced", scope: "synced"),
            row("2", name: "Cloud", section: "synced", scope: "cloudOnly"),
            row("3", name: "Empty", section: "empty", scope: "synced"),
            row("4", name: "Mine", section: "localOnly", scope: "localOnly"),
        ]
        let icloud = TodoRows.sections(rows: rows, source: .icloud, query: "", emptyExpanded: false)
        XCTAssertEqual(icloud.map(\.kind), [.synced, .empty, .localOnly])
        XCTAssertEqual(icloud[0].rows.map(\.id), ["1", "2"])
        XCTAssertTrue(icloud[1].collapsed, "projects without todos start closed")
        XCTAssertEqual(icloud[1].rows.count, 0)
        XCTAssertEqual(icloud[1].total, 1)

        let device = TodoRows.sections(rows: rows, source: .mac("a"), query: "", emptyExpanded: true)
        XCTAssertEqual(device[0].rows.map(\.id), ["1"], "no directory on this Mac for a cloud-only project")
        XCTAssertFalse(device[1].collapsed)
        XCTAssertEqual(device[1].rows.map(\.id), ["3"])
    }

    func testTheLocalViewHasTwoSections() {
        let rows = [row("1", section: "recent"), row("2", section: "withTodos"), row("3", section: "withTodos")]
        let sections = TodoRows.sections(rows: rows, source: .mac("a"), query: "", emptyExpanded: false)
        XCTAssertEqual(sections.map(\.kind), [.withTodos, .recent])
        XCTAssertEqual(sections[0].rows.map(\.id), ["2", "3"], "the Mac's order is kept")
    }

    func testSearchNeedsEveryWordAndOpensTheClosedSection() {
        let rows = [
            row("1", name: "AskHuman", section: "withTodos", hint: "work"),
            row("2", name: "AskHuman", section: "withTodos", hint: "playground"),
            row("3", name: "blog", section: "empty", scope: "synced", repo: "github.com/naituw/blog", macs: ["Home MacBook"]),
        ]
        func ids(_ query: String) -> [String] {
            TodoRows.sections(rows: rows, source: .icloud, query: query, emptyExpanded: false).flatMap(\.rows).map(\.id)
        }
        XCTAssertEqual(ids("askhuman"), ["1", "2"])
        XCTAssertEqual(ids("ask play"), ["2"], "words in any order, in the name and the hint")
        XCTAssertEqual(ids("naituw blog"), ["3"], "the repository counts, and a search opens the closed section")
        XCTAssertEqual(ids("home macbook"), ["3"], "so do the Macs that have the project")
        XCTAssertEqual(ids("nothing like it"), [])
        XCTAssertEqual(ids("  "), ["1", "2"], "blank is no search")
    }

    // MARK: sync line

    func testTheSyncLineSaysTheMostImportantThing() {
        let pulled = UInt64(1_700_000_000_000)
        func line(_ change: (inout TodoSyncInfo) -> Void) -> TodoSyncLine {
            var info = TodoSyncInfo(lastPullMs: pulled, lastPushMs: pulled - 5_000)
            change(&info)
            return TodoSyncLine.derive(info)
        }
        XCTAssertEqual(line { _ in }, .synced(at: Date(timeIntervalSince1970: 1_700_000_000)))
        XCTAssertEqual(line { $0.syncing = true }, .syncing)
        XCTAssertEqual(line { $0.unsynced = 3 }, .waiting(unsynced: 3))
        XCTAssertEqual(line { $0.paused = "notSignedIn" }, .notSignedIn)
        XCTAssertEqual(line { $0.paused = "accountChanged"; $0.syncing = true }, .accountChanged, "a stop beats activity")
        XCTAssertEqual(line { $0.helper = "notRunning"; $0.paused = "quota" }, .helperNotRunning, "no helper, no sync")
        XCTAssertEqual(line { $0.helper = "incompatible" }, .helperIncompatible)
        XCTAssertEqual(line { $0.helper = "unsupported" }, .helperUnsupported)
        XCTAssertEqual(line { $0.lastPullMs = 0; $0.lastPushMs = 0 }, .notYet)

        XCTAssertFalse(TodoSyncLine.syncing.isProblem)
        XCTAssertFalse(TodoSyncLine.waiting(unsynced: 1).isProblem)
        XCTAssertTrue(TodoSyncLine.quota.isProblem)
        XCTAssertTrue(TodoSyncLine.helperNotRunning.isProblem)
    }

    // MARK: text

    func testTextIsTrimmedAndBounded() {
        XCTAssertEqual(TodoText.validate("  buy milk \n"), .success("buy milk"))
        XCTAssertEqual(TodoText.validate(" \n "), .failure(.empty))
        XCTAssertEqual(TodoText.validate(String(repeating: "a", count: Wire.todoTextMaxChars)), .success(String(repeating: "a", count: Wire.todoTextMaxChars)))
        XCTAssertEqual(TodoText.validate(String(repeating: "a", count: Wire.todoTextMaxChars + 1)), .failure(.tooLong))
    }

    // MARK: mutations

    private func sample() -> ProjectTodos {
        ProjectTodos(
            id: "p", rev: 1, name: "P",
            pending: [TodoItem(id: "a", text: "A"), TodoItem(id: "b", text: "B", auto: true), TodoItem(id: "c", text: "C")],
            pendingTotal: 5,
            history: [TodoDoneItem(id: "h1", text: "H1", doneAtMs: 5)], historyTotal: 1
        )
    }

    private let at = Date(timeIntervalSince1970: 1_700_000_000)

    func testAddingAppendsAnEntryThatWaitsForTheMac() {
        var list = sample()
        TodoMutation.add(localId: "local-1", text: "D", auto: true).apply(to: &list, now: at)
        XCTAssertEqual(list.pending.map(\.id), ["a", "b", "c", "local-1"])
        XCTAssertTrue(list.pending.last?.auto ?? false)
        XCTAssertEqual(list.pending.last?.createdAtMs, 1_700_000_000_000)
        XCTAssertEqual(list.pendingTotal, 6, "the totals move with the list")
    }

    func testCompletingMovesTheTodoToTheFrontOfTheHistory() {
        var list = sample()
        TodoMutation.complete(id: "b").apply(to: &list, now: at)
        XCTAssertEqual(list.pending.map(\.id), ["a", "c"])
        XCTAssertEqual(list.history.map(\.id), ["b", "h1"])
        XCTAssertEqual(list.history[0].doneAtMs, 1_700_000_000_000)
        XCTAssertEqual(list.pendingTotal, 4)
        XCTAssertEqual(list.historyTotal, 2)

        TodoMutation.complete(id: "nope").apply(to: &list, now: at)
        XCTAssertEqual(list.pendingTotal, 4, "a todo that is not there changes nothing")
    }

    func testTheHistoryStaysWithinTheLimit() {
        var list = sample()
        list.history = (0..<Wire.todoMaxHistory).map { TodoDoneItem(id: "h\($0)", text: "x") }
        TodoMutation.complete(id: "a").apply(to: &list, now: at)
        XCTAssertEqual(list.history.count, Wire.todoMaxHistory)
        XCTAssertEqual(list.history.first?.id, "a")
    }

    func testRestoringPutsTheTodoAtTheEndOfTheQueue() {
        var list = sample()
        TodoMutation.restore(id: "h1").apply(to: &list, now: at)
        XCTAssertEqual(list.pending.map(\.id), ["a", "b", "c", "h1"])
        XCTAssertTrue(list.history.isEmpty)
        XCTAssertEqual(list.historyTotal, 0)
        XCTAssertEqual(list.pendingTotal, 6)
    }

    func testEditingRemovingAndFlaggingTouchOnlyTheirTodo() {
        var list = sample()
        TodoMutation.edit(id: "a", text: "A2").apply(to: &list, now: at)
        TodoMutation.setAuto(id: "c", auto: true).apply(to: &list, now: at)
        TodoMutation.remove(id: "b").apply(to: &list, now: at)
        XCTAssertEqual(list.pending.map(\.text), ["A2", "C"])
        XCTAssertEqual(list.pending.map(\.auto), [false, true])
        XCTAssertEqual(list.pendingTotal, 4)
    }

    func testReorderingKeepsTheTodosThatWereNotListedAfterTheOnesThatWere() {
        var list = sample()
        TodoMutation.reorder(ids: ["c", "a", "zzz"]).apply(to: &list, now: at)
        XCTAssertEqual(list.pending.map(\.id), ["c", "a", "b"])
    }

    func testEachMutationBecomesTheMatchingOperation() {
        func op(_ mutation: TodoMutation) -> TodoOperation { mutation.operation(clientMsgId: "m", projectId: "p") }
        XCTAssertEqual(op(.add(localId: "l", text: "T", auto: false)), TodoOperation(clientMsgId: "m", id: "p", op: "add", text: "T"))
        XCTAssertEqual(op(.add(localId: "l", text: "T", auto: true)), TodoOperation(clientMsgId: "m", id: "p", op: "add", text: "T", auto: true))
        XCTAssertEqual(op(.edit(id: "a", text: "T")), TodoOperation(clientMsgId: "m", id: "p", op: "edit", todoId: "a", text: "T"))
        XCTAssertEqual(op(.complete(id: "a")), TodoOperation(clientMsgId: "m", id: "p", op: "complete", todoId: "a"))
        XCTAssertEqual(op(.restore(id: "a")), TodoOperation(clientMsgId: "m", id: "p", op: "restore", todoId: "a"))
        XCTAssertEqual(op(.remove(id: "a")), TodoOperation(clientMsgId: "m", id: "p", op: "remove", todoId: "a"))
        XCTAssertEqual(op(.setAuto(id: "a", auto: false)), TodoOperation(clientMsgId: "m", id: "p", op: "auto", todoId: "a", auto: false))
        XCTAssertEqual(op(.reorder(ids: ["b", "a"])), TodoOperation(clientMsgId: "m", id: "p", op: "reorder", order: ["b", "a"]))
    }

    // MARK: moving rows

    @MainActor
    func testMovingRowsFollowsOnMoveSemantics() {
        let ids = ["a", "b", "c", "d"]
        XCTAssertEqual(TodosStore.moved(ids, from: IndexSet(integer: 0), to: 3), ["b", "c", "a", "d"], "to before the element at 3")
        XCTAssertEqual(TodosStore.moved(ids, from: IndexSet(integer: 3), to: 0), ["d", "a", "b", "c"])
        XCTAssertEqual(TodosStore.moved(ids, from: IndexSet(integer: 1), to: 4), ["a", "c", "d", "b"], "to the very end")
        XCTAssertEqual(TodosStore.moved(ids, from: IndexSet([0, 1]), to: 4), ["c", "d", "a", "b"])
        XCTAssertEqual(TodosStore.moved(ids, from: IndexSet(integer: 2), to: 2), ids, "dropped where it was")
    }
}
