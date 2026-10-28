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
