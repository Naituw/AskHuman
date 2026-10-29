import XCTest
@testable import AHProtocol
@testable import AHCore

/// The dots of the Todos tab (docs/specs/ios-todos.md I18): the Agents tab's states, joined to the project
/// rows by repository, or by the directory on the same Mac.
final class TodoActivityTests: XCTestCase {
    private func agent(
        _ key: String, mac: String = "a", state: String, path: String = "/work/app", repo: String? = nil
    ) -> AgentItem {
        AgentItem(
            macId: mac, macName: "Mac " + mac,
            view: AgentView(
                key: key, kind: "claude", kindLabel: "Claude Code", sessionId: key, title: key,
                project: ProjectRef(key: path, name: "app", repo: repo), state: state, startedAt: 0, lastActivityAt: 0,
                activeElapsedSecs: 0, turnSteps: 0, canInterject: CanInterject(ok: true), followed: false
            )
        )
    }

    private func project(
        _ key: String, mac: String = "a", repo: String? = nil, scope: String = "synced"
    ) -> TodoListedProject {
        TodoListedProject(
            macId: mac, macName: "Mac " + mac,
            row: TodoProjectRow(id: TodoProjectId.of(key: key), name: "app", repo: repo, scope: scope)
        )
    }

    func testTheIdsAreTheOnesTheMacCalculates() {
        // `todos_view::project_ids_match_the_ones_the_phone_computes` pins the same values.
        XCTAssertEqual(TodoProjectId.of(key: "/tmp/proj"), "29ccbcbce9441d72")
        XCTAssertEqual(TodoProjectId.of(key: "/Users/me/askhuman"), "b044499166be822c")
    }

    func testTheStatesAreTheAgentsTabsAndAnEndedSessionSaysNothing() {
        XCTAssertEqual(TodoActivity(agentState: "waiting"), .waiting)
        XCTAssertEqual(TodoActivity(agentState: "working"), .working)
        XCTAssertEqual(TodoActivity(agentState: "idle"), .idle)
        XCTAssertEqual(TodoActivity(agentState: "something new"), .idle, "a state this phone does not know is not a reason to hide the project")
        XCTAssertNil(TodoActivity(agentState: "ended"))
        XCTAssertTrue(TodoActivity.idle < .working && TodoActivity.working < .waiting)
    }

    func testAnAgentOnAnotherMacMarksTheProjectOfTheSameRepository() {
        let index = TodoActivityIndex(agents: [
            agent("s1", mac: "b", state: "working", path: "/Users/other/app-wt", repo: "github.com/acme/app")
        ])
        // The row is the access Mac's own directory of the repository: no path in common with the Agent's.
        XCTAssertEqual(index.activity(of: project("/work/app", mac: "a", repo: "github.com/acme/app")), .working)
        XCTAssertNil(index.activity(of: project("/work/app", mac: "a", repo: "github.com/acme/other")))
    }

    func testTheMostUrgentStateOfAProjectWins() {
        let repo = "github.com/acme/app"
        func state(_ states: [String]) -> TodoActivity? {
            let agents = states.enumerated().map { agent("s\($0.offset)", state: $0.element, repo: repo) }
            return TodoActivityIndex(agents: agents).activity(of: project("/work/app", repo: repo))
        }
        XCTAssertEqual(state(["idle", "working"]), .working)
        XCTAssertEqual(state(["working", "waiting", "idle"]), .waiting)
        XCTAssertEqual(state(["idle", "idle"]), .idle)
        XCTAssertNil(state(["ended"]), "an ended session leaves no dot")
        XCTAssertEqual(state(["ended", "idle"]), .idle)
        XCTAssertNil(state([]))
    }

    func testAProjectWithoutARepositoryIsFoundByItsDirectoryOnTheSameMac() {
        let index = TodoActivityIndex(agents: [agent("s1", mac: "a", state: "waiting", path: "/work/scratch")])
        XCTAssertEqual(index.activity(of: project("/work/scratch", mac: "a", scope: "localOnly")), .waiting)
        XCTAssertNil(index.activity(of: project("/work/scratch", mac: "b", scope: "localOnly")), "the same path on another Mac is another project")
        XCTAssertNil(index.activity(of: project("/work/other", mac: "a", scope: "localOnly")))
    }

    func testBothWaysCountAndTheMoreUrgentOneWins() {
        let repo = "github.com/acme/app"
        let index = TodoActivityIndex(agents: [
            agent("s1", mac: "a", state: "idle", path: "/work/app", repo: repo),
            agent("s2", mac: "b", state: "working", path: "/elsewhere/app", repo: repo),
        ])
        XCTAssertEqual(index.activity(of: project("/work/app", mac: "a", repo: repo)), .working)
    }

    func testAnOldMacThatSendsNoRepositoryStillMarksItsOwnDirectories() {
        let index = TodoActivityIndex(agents: [agent("s1", state: "working", path: "/work/app", repo: nil)])
        XCTAssertEqual(index.activity(of: project("/work/app", mac: "a", repo: "github.com/acme/app")), .working)
        XCTAssertNil(index.activity(of: project("/other/app", mac: "a", repo: "github.com/acme/app")))
    }

    func testAnAgentWithoutAProjectMarksNothing() {
        let index = TodoActivityIndex(agents: [agent("s1", state: "working", path: "", repo: "")])
        XCTAssertTrue(index.isEmpty)
    }
}
