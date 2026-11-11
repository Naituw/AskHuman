import XCTest
@testable import AHProtocol

/// The earlier exchanges of a session on the wire (docs/specs/ios-session-history.md §5). Everything is additive:
/// a Mac and a phone of different ages exchange exactly what each of them knows.
final class SessionHistoryWireTests: XCTestCase {
    private func caps(sessionHistory: Bool) -> Caps {
        Caps(newTask: true, interject: true, watch: true, todos: true, answerTodos: true, away: true, sessionHistory: sessionHistory)
    }

    func testCapsLeaveTheHistoryBitOutWhenFalse() throws {
        XCTAssertNil(try AHJSON.tree(caps(sessionHistory: false))["sessionHistory"], "what predates it sees what it always saw")
        XCTAssertEqual(try AHJSON.tree(caps(sessionHistory: true))["sessionHistory"], .bool(true))
    }

    func testAMissingHistoryBitReadsAsNo() throws {
        let old = try AHJSON.decoder.decode(Caps.self, from: Data(#"{"newTask":true,"interject":true,"watch":true,"todos":true}"#.utf8))
        XCTAssertFalse(old.sessionHistory)
        let new = try AHJSON.decoder.decode(
            Caps.self, from: Data(#"{"newTask":true,"interject":true,"watch":true,"sessionHistory":true}"#.utf8)
        )
        XCTAssertTrue(new.sessionHistory)
        for flag in [false, true] {
            XCTAssertEqual(try AHJSON.value(Caps.self, from: try AHJSON.tree(caps(sessionHistory: flag))), caps(sessionHistory: flag))
        }
    }

    func testARequestCarriesTheSessionKeyOnlyWhenThereIsOne() throws {
        func view(_ key: String?) -> RequestView {
            RequestView(
                id: "r", kind: "ask", createdAt: 1, agent: AgentRef(kind: "claude", label: "Claude Code"),
                project: ProjectRef(key: "k", name: "p"), agentKey: key, source: "mcp", contentHash: "h"
            )
        }
        XCTAssertNil(try AHJSON.tree(view(nil))["agentKey"], "an older phone is not sent a field it never read")
        XCTAssertEqual(try AHJSON.tree(view("claude:s1"))["agentKey"], .string("claude:s1"))
        for key in [nil, "claude:s1"] {
            XCTAssertEqual(try AHJSON.value(RequestView.self, from: try AHJSON.tree(view(key))), view(key))
        }
        guard case .object(var fields) = try AHJSON.tree(view("claude:s1")) else { return XCTFail("an object") }
        fields["agentKey"] = nil
        XCTAssertNil(try AHJSON.value(RequestView.self, from: .object(fields)).agentKey, "a request from an older Mac has none")
    }

    func testTheAskLeavesOutWhatIsNotThere() throws {
        let get = SessionHistoryGet(reqId: "h1", agentKey: "claude:s1", beforeMs: 5, limit: 8)
        XCTAssertEqual(
            try AHJSON.tree(get),
            .object(["reqId": .string("h1"), "agentKey": .string("claude:s1"), "beforeMs": .int(5), "limit": .int(8)])
        )
        let again = SessionHistoryGet(reqId: "h2", agentKey: "claude:s1", beforeMs: 5, beforeId: "req-8", limit: 20)
        XCTAssertEqual(try AHJSON.tree(again)["beforeId"], .string("req-8"))
        XCTAssertEqual(try AHJSON.value(SessionHistoryGet.self, from: try AHJSON.tree(again)), again)
    }

    func testAnExchangeLeavesOutCountsAndFlagsThatAreZero() throws {
        let bare = SessionExchange(
            id: "r", at: 1, channel: "away", message: "", qs: [SessionQuestion(text: "ok?")]
        )
        XCTAssertEqual(
            try AHJSON.tree(bare),
            .object([
                "id": .string("r"), "at": .int(1), "channel": .string("away"), "message": .string(""),
                "qs": .array([.object(["text": .string("ok?")])]),
            ])
        )
        let full = SessionExchange(
            id: "r2", at: 2, channel: "popup", markdown: true, message: "m", attachments: 1,
            qs: [SessionQuestion(text: "q", selected: ["a"], input: "words", images: 1, files: 2)], notes: 3
        )
        let tree = try AHJSON.tree(full)
        XCTAssertEqual(tree["markdown"], .bool(true))
        XCTAssertEqual(tree["attachments"], .int(1))
        XCTAssertEqual(tree["notes"], .int(3))
        XCTAssertEqual(try AHJSON.value(SessionExchange.self, from: tree), full)
        // A reader sees the defaults when the Mac left them out.
        let decoded = try AHJSON.value(SessionExchange.self, from: try AHJSON.tree(bare))
        XCTAssertFalse(decoded.markdown)
        XCTAssertEqual(decoded.attachments, 0)
        XCTAssertEqual(decoded.notes, 0)
        XCTAssertEqual(decoded.qs.first?.selected, [])
        XCTAssertNil(decoded.qs.first?.input)
    }

    func testThePageKeepsItsOrderAndSaysWhetherThereIsMore() throws {
        let page = SessionHistory(
            reqId: "h2", agentKey: "claude:s1",
            items: [
                SessionExchange(id: "new", at: 20, channel: "ios", message: "b", qs: []),
                SessionExchange(id: "old", at: 10, channel: "popup", message: "a", qs: []),
            ],
            more: true
        )
        let back = try AHJSON.value(SessionHistory.self, from: try AHJSON.tree(page))
        XCTAssertEqual(back.items.map(\.id), ["new", "old"], "newest first, as the Mac sent it")
        XCTAssertTrue(back.more)
        XCTAssertEqual(try AHJSON.tree(SessionHistory(reqId: "h", agentKey: "k"))["more"], .bool(false), "always sent")
    }

    func testTheLimitsAreTheMacs() {
        // `SESSION_HISTORY_*` and `HISTORY_*` in `src-tauri/src/ios/wire.rs`.
        XCTAssertEqual(Wire.sessionHistoryPage, 8)
        XCTAssertEqual(Wire.sessionHistoryPageMax, 20)
        XCTAssertEqual(Wire.sessionHistoryBytes, 256 * 1024)
        XCTAssertEqual(Wire.historyMessageMaxChars, 4000)
        XCTAssertEqual(Wire.historyQuestionMaxChars, 1000)
        XCTAssertEqual(Wire.historyOptionMaxChars, 500)
        XCTAssertEqual(Wire.historyInputMaxChars, 2000)
    }
}
