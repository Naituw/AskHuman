import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// A session's earlier exchanges (docs/specs/ios-session-history.md): the Swift phone engine against the real Rust gateway
/// in a real daemon, reading the throwaway Mac's own `history.jsonl`. Every exchange here is made up.
///
///   AH_E2E_BIN=<installed AskHuman> AH_E2E_BROKER_DIR=ios/Tools/local-broker swift test --filter SessionHistoryE2ETests
final class SessionHistoryE2ETests: E2ECase {
    private let session = "e2e-history-session"

    // MARK: helpers

    private func macId() async throws -> String { try unwrap(await phone.state().macs.first?.id) }

    /// One recorded reply, as a line of `history.jsonl`.
    private func entry(_ id: String, kind: String?, session: String?, at ms: Int64, question: String? = nil) throws -> String {
        var object: [String: Any] = [
            "id": id, "timestampMs": ms, "project": "/made/up/project", "source": "E2E", "channel": "popup", "action": "send",
            "isMarkdown": false, "message": ["text": "Message of \(id)", "files": []],
            "questions": [["message": question ?? "Question of \(id)?", "predefinedOptions": [["text": "Yes", "recommended": false]]]],
            "answers": [["selectedOptions": ["Yes"], "images": [], "files": []]],
        ]
        if let kind { object["agentKind"] = kind }
        if let session { object["agentSessionId"] = session }
        let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }

    private func writeHistory(_ lines: [String]) throws {
        try (lines.joined(separator: "\n") + "\n")
            .write(to: mac.configDir.appendingPathComponent("history.jsonl"), atomically: true, encoding: .utf8)
    }

    private func page(
        before ms: Int64, id: String? = nil, limit: Int = Wire.sessionHistoryPage, key: String? = nil
    ) async throws -> SessionHistory {
        try await phone.engine.sessionHistory(
            macId: try await macId(), agentKey: key ?? "claude:\(session)", beforeMs: ms, beforeId: id, limit: limit
        )
    }

    // MARK: the pages

    /// Twelve exchanges of one session, others of other sessions around them: the phone gets its own session's, newest
    /// first, eight and then four, and never a line of another one.
    func testASessionsExchangesComeNewestFirstInPagesAndAlone() async throws {
        try await pair()
        let base = Int64(Date().timeIntervalSince1970 * 1000) - 3_600_000
        var lines = [String]()
        for number in 1...12 {
            lines.append(try entry("own-\(number)", kind: "claude", session: session, at: base + Int64(number) * 1_000))
        }
        lines.append(try entry("other-session", kind: "claude", session: "another-session", at: base + 5_500))
        lines.append(try entry("other-agent", kind: "codex", session: session, at: base + 6_500))
        lines.append(try entry("no-session", kind: nil, session: nil, at: base + 7_500))
        try writeHistory(lines)

        let anchor = base + 100_000
        let first = try await page(before: anchor)
        XCTAssertEqual(first.items.map(\.id), (5...12).reversed().map { "own-\($0)" }, "the newest eight, newest first")
        XCTAssertTrue(first.more)
        XCTAssertEqual(first.agentKey, "claude:\(session)")
        XCTAssertEqual(first.items.first?.message, "Message of own-12")
        XCTAssertEqual(first.items.first?.qs.first?.text, "Question of own-12?")
        XCTAssertEqual(first.items.first?.qs.first?.selected, ["Yes"])

        let oldest = try unwrap(first.items.last)
        let second = try await page(before: oldest.at, id: oldest.id)
        XCTAssertEqual(second.items.map(\.id), (1...4).reversed().map { "own-\($0)" }, "the rest, without repeating a line of the first")
        XCTAssertFalse(second.more, "that was the start of the session")

        // Nothing after the cursor: a request asked at that moment has no later exchange for a history.
        let early = try await page(before: base + 3_500, limit: 20)
        XCTAssertEqual(early.items.map(\.id), ["own-3", "own-2", "own-1"])
        XCTAssertFalse(early.more)
    }

    /// Two answers in the very same millisecond are neither skipped nor shown twice from one page to the next.
    func testAnswersOfTheSameMillisecondSurviveThePageBoundary() async throws {
        try await pair()
        let moment = Int64(Date().timeIntervalSince1970 * 1000) - 600_000
        let ids = (1...5).map { "same-ms-\($0)" }
        try writeHistory(try ids.map { try entry($0, kind: "claude", session: session, at: moment) })

        let first = try await page(before: moment + 1, limit: 2)
        XCTAssertEqual(first.items.count, 2)
        XCTAssertTrue(first.more)
        var seen = first.items.map(\.id)
        var cursor = try unwrap(first.items.last)
        while true {
            let next = try await page(before: cursor.at, id: cursor.id, limit: 2)
            seen += next.items.map(\.id)
            guard next.more, let last = next.items.last else { break }
            cursor = last
        }
        XCTAssertEqual(seen.count, 5, "all five, once each: \(seen)")
        XCTAssertEqual(Set(seen), Set(ids))
    }

    func testASessionWithoutRecordsGivesAnEmptyPageNotAnError() async throws {
        try await pair()
        try writeHistory([try entry("elsewhere", kind: "claude", session: "another-session", at: 1_000)])
        let empty = try await page(before: Int64(Date().timeIntervalSince1970 * 1000))
        XCTAssertTrue(empty.items.isEmpty)
        XCTAssertFalse(empty.more)
        let unreadable = try await page(before: 1_000, key: "no-colon-in-this-key")
        XCTAssertTrue(unreadable.items.isEmpty, "a key the Mac cannot read is an empty page too")
    }

    // MARK: the key of a request

    /// A request of a real Agent session names it exactly as the Agents list does, so the phone can ask for its history.
    func testARequestOfARealSessionCarriesTheKeyOfItsAgentRow() async throws {
        try await pair()
        let payload = try JSONSerialization.data(withJSONObject: ["session_id": session, "cwd": mac.home.path])
        let started = try await mac.cli(
            ["__agent-hook", "claude", "session-start"], stdin: String(decoding: payload, as: UTF8.self), timeout: 30
        )
        XCTAssertEqual(started.status, 0, started.stderr)
        await eventually("the session in the Agents list") {
            await self.phone.state().agents.contains { $0.agentKey == "claude:\(self.session)" }
        }

        // What a Claude Code shell tool gives the processes it starts.
        var env = mac.env
        env["CLAUDECODE"] = "1"
        env["CLAUDE_CODE_SESSION_ID"] = session
        let ask = Cmd(mac.environment.askHumanBinary, ["Which one?", "-o", "Yes", "-o", "No"], env: env, cwd: mac.home)
        try ask.start()
        defer { ask.terminate() }

        let item = try await phone.request()
        XCTAssertEqual(item.view?.agentKey, "claude:\(session)")
        let row = try unwrap(await phone.state().agents.first { $0.agentKey == "claude:\(session)" })
        XCTAssertEqual(item.view?.agentKey, row.agentKey, "the same key in the list and on the request")
    }

    /// A request that names no session has no key: the phone offers no history for it.
    func testARequestWithoutASessionHasNoKey() async throws {
        try await pair()
        let ask = try mac.ask(["Which one?", "-o", "Yes", "-o", "No"])
        let item = try await phone.request()
        XCTAssertNil(item.view?.agentKey)
        ask.terminate()
    }
}
