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
        XCTAssertTrue(item(state: "idle", outcome: "failed").lastTurnFellShort)
        XCTAssertTrue(item(state: "ended", outcome: "incomplete").lastTurnFellShort)
        XCTAssertFalse(item(state: "idle", outcome: "interrupted").lastTurnFellShort)
        XCTAssertFalse(item(state: "idle", outcome: "unknown").lastTurnFellShort)
        XCTAssertFalse(item(state: "idle", outcome: "completed").lastTurnFellShort)
    }

    func testARunningOrWaitingSessionDoesNotShowItsOldResult() {
        XCTAssertFalse(item(state: "working", outcome: "failed").showsLastResult)
        XCTAssertFalse(item(state: "waiting", outcome: "failed").showsLastResult)
        XCTAssertTrue(item(state: "idle", outcome: "failed").showsLastResult)
        XCTAssertTrue(item(state: "ended", outcome: "failed").showsLastResult, "a session that ended still reports how its last turn went")
    }

    func testResultsSurviveWireRoundTripsAndFutureOutcomesStayReadable() throws {
        let view = item(state: "idle", outcome: "futureOutcome").view
        let encoded = try JSONEncoder().encode(view)
        let decoded = try JSONDecoder().decode(AgentView.self, from: encoded)
        XCTAssertEqual(decoded.lastTurnResult?.outcome, "futureOutcome")
        XCTAssertEqual(decoded.state, "idle")
        XCTAssertFalse(AgentItem(macId: "mac", macName: "Mac", view: decoded).lastTurnFellShort)
    }
}
