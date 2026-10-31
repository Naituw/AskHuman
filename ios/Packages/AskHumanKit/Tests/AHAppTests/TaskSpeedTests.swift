import XCTest
import AHCore
import AHProtocol

@MainActor
final class TaskSpeedTests: XCTestCase {
    private func options(control: Bool? = true) -> TaskOptions {
        TaskOptions(enabled: true, permissionMode: "agentDefault", workspaces: [
            WorkspaceOption(id: "w", label: "Project", subtitle: "", pinned: true, lastUsedAt: 0)
        ], agents: [AgentOption(kind: "codex", label: "Codex", ready: true, subtitle: "", permissionSelectable: false,
            models: [ModelOption(id: "quick", label: "Quick", efforts: ["high"], fastSupported: true),
                     ModelOption(id: "other", label: "Other", fastSupported: true),
                     ModelOption(id: "slow", label: "Slow", fastSupported: false)], speedControl: control),
            AgentOption(kind: "claude", label: "Claude", ready: true, subtitle: "", permissionSelectable: false)])
    }

    private func form(_ choices: TaskOptions, defaults: UserDefaults = UserDefaults(suiteName: UUID().uuidString)!) -> TaskComposer {
        let form = TaskComposer(macId: "mac")
        form.applyDefaults(options: choices, defaults: defaults)
        form.text = "Do it"
        return form
    }

    func testOffExplicitlySendsStandardEvenWithDefaultModelAndEffort() throws {
        let choices = options(), form = form(options())
        XCTAssertFalse(form.fastMode)
        XCTAssertEqual(try XCTUnwrap(form.draft(options: choices)).speed, .standard)
        XCTAssertFalse(form.supportsFast(in: choices), "an unresolved default model is not evidence")
        form.selectModel("quick", in: choices)
        form.effortId = "high"
        form.setFastMode(true, in: choices)
        let draft = try XCTUnwrap(form.draft(options: choices))
        XCTAssertEqual(draft.speed, .fast)
        XCTAssertEqual(draft.effort, "high")
        form.setFastMode(false, in: choices)
        XCTAssertEqual(form.draft(options: choices)?.speed, .standard)
    }

    func testFastIsNeverRememberedButModelAndEffortStillAre() {
        let choices = options(), defaults = UserDefaults(suiteName: UUID().uuidString)!
        let first = form(choices, defaults: defaults)
        first.selectModel("quick", in: choices)
        first.effortId = "high"
        first.setFastMode(true, in: choices)
        first.remember(options: choices, defaults: defaults)
        let next = form(choices, defaults: defaults)
        XCTAssertEqual(next.modelId, "quick")
        XCTAssertEqual(next.effortId, "high")
        XCTAssertFalse(next.fastMode)
        XCTAssertEqual(next.draft(options: choices)?.speed, .standard)
    }

    func testModelChangesKeepFastOnlyWhileSupportedAndMacOrAgentChangesResetIt() {
        let choices = options(), form = form(options())
        form.selectModel("quick", in: choices)
        form.setFastMode(true, in: choices)
        form.selectModel("other", in: choices)
        XCTAssertTrue(form.fastMode)
        form.selectModel("slow", in: choices)
        XCTAssertFalse(form.fastMode)
        XCTAssertTrue(form.fastWasReset)
        form.selectModel("quick", in: choices)
        XCTAssertFalse(form.fastMode)
        form.setFastMode(true, in: choices)
        form.selectAgent("claude")
        XCTAssertFalse(form.fastMode)
        form.selectAgent("codex")
        form.selectModel("quick", in: choices)
        form.setFastMode(true, in: choices)
        form.switchMac(to: "other")
        XCTAssertFalse(form.fastMode)
    }

    func testCapabilityLossClearsFastAndOldMacGetsNoSpeedOverride() {
        let choices = options(), form = form(options())
        form.selectModel("quick", in: choices)
        form.setFastMode(true, in: choices)
        let old = options(control: nil)
        form.applyDefaults(options: old, defaults: UserDefaults(suiteName: UUID().uuidString)!)
        XCTAssertFalse(form.fastMode)
        XCTAssertFalse(form.offersSpeedControl(in: old))
        XCTAssertNil(form.draft(options: old)?.speed)
    }

    func testWatchSpeedOnlyUsesExplicitKnownConfiguration() throws {
        let known = try XCTUnwrap(WatchFrameData(json: .object(["phase": .string("working"), "speed": .string("fast")])))
        XCTAssertEqual(known.speed, .fast)
        XCTAssertNil(WatchFrameData(json: .object(["phase": .string("working")]))?.speed)
        XCTAssertNil(WatchFrameData(json: .object(["speed": .string("ultrafast")]))?.speed)
    }
}
