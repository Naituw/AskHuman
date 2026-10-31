import XCTest
import AHProtocol
import AHCore

final class AgentResultTests: XCTestCase {
    private func item(state: String, outcome: String?) -> AgentItem {
        AgentItem(macId: "mac", macName: "Mac", view: AgentView(
            key: "cursor:s", kind: "cursor", kindLabel: "Cursor", sessionId: "s", title: "Task",
            project: ProjectRef(key: "p", name: "Project"), state: state, startedAt: 0, lastActivityAt: 0,
            lastTurnResult: outcome.map { AgentTurnResult(outcome: $0) }
        ))
    }

    func testResultDoesNotReplaceTheRuntimeState() {
        XCTAssertTrue(item(state: "idle", outcome: "failed").needsAttention)
        XCTAssertTrue(item(state: "ended", outcome: "incomplete").needsAttention)
        XCTAssertFalse(item(state: "working", outcome: "failed").needsAttention)
        XCTAssertFalse(item(state: "waiting", outcome: "failed").needsAttention)
        XCTAssertFalse(item(state: "idle", outcome: "interrupted").needsAttention)
        XCTAssertFalse(item(state: "idle", outcome: "unknown").needsAttention)
    }

    func testResultsSurviveWireRoundTripsAndFutureOutcomesStayReadable() throws {
        let view = item(state: "idle", outcome: "futureOutcome").view
        let encoded = try JSONEncoder().encode(view)
        let decoded = try JSONDecoder().decode(AgentView.self, from: encoded)
        XCTAssertEqual(decoded.lastTurnResult?.outcome, "futureOutcome")
        XCTAssertEqual(decoded.state, "idle")
        XCTAssertFalse(AgentItem(macId: "mac", macName: "Mac", view: decoded).needsAttention)
    }
}
