import XCTest
import AHProtocol
@testable import AHCore

@MainActor
final class TaskComposerTests: XCTestCase {
    private func options(
        mode: String = "ask", selectable: Bool = true, ready: Bool = true, enabled: Bool = true, workspaces: Int = 2
    ) -> TaskOptions {
        TaskOptions(
            enabled: enabled, permissionMode: mode,
            workspaces: (0..<workspaces).map { WorkspaceOption(id: "ws\($0)", label: "Project \($0)", subtitle: "~/p\($0)", pinned: $0 == 1) },
            agents: [
                AgentOption(kind: "cursor", label: "Cursor", ready: false, subtitle: "", permissionSelectable: false, reason: "Not installed"),
                AgentOption(kind: "claude", label: "Claude Code", ready: ready, subtitle: "", permissionSelectable: selectable),
            ]
        )
    }

    private func defaults() -> UserDefaults { UserDefaults(suiteName: "ah-test-" + UUID().uuidString)! }

    func testDefaultsPreferThePinnedWorkspaceAndTheFirstReadyAgent() {
        let composer = TaskComposer(macId: "mac")
        composer.applyDefaults(options: options(), defaults: defaults())
        XCTAssertEqual(composer.workspaceId, "ws1")
        XCTAssertEqual(composer.agentKind, "claude")
    }

    func testTheLastChoicesOfAMacAreRememberedWhenTheyAreStillOffered() {
        let store = defaults()
        let first = TaskComposer(macId: "mac")
        first.applyDefaults(options: options(), defaults: store)
        first.workspaceId = "ws0"
        first.remember(defaults: store)

        let second = TaskComposer(macId: "mac")
        second.applyDefaults(options: options(), defaults: store)
        XCTAssertEqual(second.workspaceId, "ws0")

        let other = TaskComposer(macId: "other-mac")
        other.applyDefaults(options: options(), defaults: store)
        XCTAssertEqual(other.workspaceId, "ws1", "another Mac has its own memory")

        let gone = TaskComposer(macId: "mac")
        gone.applyDefaults(options: options(workspaces: 1), defaults: store)
        XCTAssertEqual(gone.workspaceId, "ws0")
    }

    func testAnInvalidSelectionIsReplacedButAValidOneStays() {
        let composer = TaskComposer(macId: "mac")
        composer.workspaceId = "vanished"
        composer.agentKind = "claude"
        composer.applyDefaults(options: options(), defaults: defaults())
        XCTAssertEqual(composer.workspaceId, "ws1")
        XCTAssertEqual(composer.agentKind, "claude")
    }

    func testThePermissionFollowsTheMacsSettingAndTheAgent() {
        let composer = TaskComposer(macId: "mac")
        composer.applyDefaults(options: options(), defaults: defaults())
        composer.permissionChoice = .yolo

        XCTAssertTrue(composer.offersPermissionChoice(in: options(mode: "ask")))
        XCTAssertEqual(composer.permission(in: options(mode: "ask")), "yolo")
        XCTAssertFalse(composer.offersPermissionChoice(in: options(mode: "agentDefault")))
        XCTAssertEqual(composer.permission(in: options(mode: "agentDefault")), "default", "the Mac's fixed setting wins")
        XCTAssertEqual(composer.permission(in: options(mode: "yolo")), "yolo")
        XCTAssertFalse(composer.offersPermissionChoice(in: options(mode: "ask", selectable: false)))
        XCTAssertEqual(composer.permission(in: options(mode: "ask", selectable: false)), "default", "an Agent without a mode cannot skip prompts")
    }

    func testWhenTheMacAsksTheChoiceIsNotPreselectedAndNeededToSubmit() {
        let composer = TaskComposer(macId: "mac")
        let asking = options(mode: "ask")
        composer.applyDefaults(options: asking, defaults: defaults())
        composer.text = "Fix the build"
        XCTAssertNil(composer.permissionChoice, "nothing is preselected")
        XCTAssertFalse(composer.canSubmit(options: asking), "the user has to choose")
        XCTAssertNil(composer.draft(options: asking))

        composer.permissionChoice = .standard
        XCTAssertTrue(composer.canSubmit(options: asking))
        XCTAssertEqual(composer.draft(options: asking)?.permission, "default")
        composer.permissionChoice = .yolo
        XCTAssertEqual(composer.draft(options: asking)?.permission, "yolo")

        XCTAssertNotNil(composer.draft(options: options(mode: "agentDefault")), "a fixed setting needs no choice")
        composer.permissionChoice = nil
        XCTAssertTrue(composer.canSubmit(options: options(mode: "ask", selectable: false)), "an Agent without a mode needs no choice")
    }

    func testAnotherAgentAsksForTheChoiceAgain() {
        let composer = TaskComposer(macId: "mac")
        composer.applyDefaults(options: options(), defaults: defaults())
        composer.permissionChoice = .yolo
        composer.selectAgent(composer.agentKind ?? "")
        XCTAssertEqual(composer.permissionChoice, .yolo, "the same Agent keeps it")
        composer.selectAgent("codex")
        XCTAssertNil(composer.permissionChoice)
    }

    func testSubmittingNeedsATextOrATodoAndAnAgentThatIsReady() {
        let composer = TaskComposer(macId: "mac")
        let choices = options()
        composer.applyDefaults(options: choices, defaults: defaults())
        composer.permissionChoice = .standard
        XCTAssertFalse(composer.canSubmit(options: choices), "nothing to do yet")
        composer.text = "  \n "
        XCTAssertFalse(composer.canSubmit(options: choices), "blank text does not count")
        composer.text = "Fix the build"
        XCTAssertTrue(composer.canSubmit(options: choices))
        XCTAssertFalse(composer.canSubmit(options: options(ready: false)), "the Agent is not ready")
        XCTAssertFalse(composer.canSubmit(options: options(enabled: false)), "the Mac does not allow tasks")

        composer.text = ""
        composer.todoId = "todo-1"
        XCTAssertTrue(composer.canSubmit(options: choices), "a TODO alone is enough")
    }

    func testTheTextIsLimitedLikeOnTheMac() {
        let composer = TaskComposer(macId: "mac")
        let choices = options()
        composer.applyDefaults(options: choices, defaults: defaults())
        composer.permissionChoice = .standard
        composer.text = String(repeating: "a", count: TaskComposer.maxChars)
        XCTAssertFalse(composer.overLimit)
        XCTAssertTrue(composer.canSubmit(options: choices))
        composer.text += "a"
        XCTAssertTrue(composer.overLimit)
        XCTAssertFalse(composer.canSubmit(options: choices))
        XCTAssertNil(composer.draft(options: choices))
    }

    func testTheDraftCarriesWhatTheUserChose() throws {
        let composer = TaskComposer(macId: "mac")
        let choices = options()
        composer.applyDefaults(options: choices, defaults: defaults())
        composer.permissionChoice = .standard
        composer.text = "  Fix the build \n"
        composer.todoId = "todo-9"
        let draft = try XCTUnwrap(composer.draft(options: choices))
        XCTAssertEqual(draft, TaskDraft(
            workspaceId: "ws1", workspaceName: "Project 1", agentKind: "claude", agentLabel: "Claude Code", permission: "default",
            todoId: "todo-9", text: "Fix the build"
        ))
    }

    func testSwitchingTheMacForgetsItsChoicesButNotTheText() {
        let composer = TaskComposer(macId: "mac")
        composer.applyDefaults(options: options(), defaults: defaults())
        composer.text = "Keep"
        composer.todoId = "t"
        composer.permissionChoice = .yolo
        composer.switchMac(to: "other")
        XCTAssertEqual(composer.macId, "other")
        XCTAssertNil(composer.workspaceId)
        XCTAssertNil(composer.agentKind)
        XCTAssertNil(composer.todoId)
        XCTAssertNil(composer.permissionChoice)
        XCTAssertEqual(composer.text, "Keep")
    }
}
