import XCTest
import AHCore
import AHProtocol

/// Cursor's Max mode on the new task form (`docs/specs/cursor-max-control.md` §5): off for every task,
/// and a model that cannot run without it does not start unless the task allows it.
@MainActor
final class TaskMaxTests: XCTestCase {
    private func options(control: Bool? = true, defaultNeedsMax: Bool? = true) -> TaskOptions {
        TaskOptions(enabled: true, permissionMode: "agentDefault", workspaces: [
            WorkspaceOption(id: "w", label: "Project", subtitle: "", pinned: true, lastUsedAt: 0)
        ], agents: [
            AgentOption(
                kind: "cursor", label: "Cursor", ready: true, subtitle: "", permissionSelectable: false,
                models: [
                    ModelOption(id: "small", label: "Small", maxRequired: false),
                    ModelOption(id: "big", label: "Big", maxRequired: true),
                    ModelOption(id: "fresh", label: "Fresh"),
                ],
                defaultModel: DefaultModel(label: "Big", maxRequired: defaultNeedsMax), maxControl: control
            ),
            AgentOption(
                kind: "claude", label: "Claude", ready: true, subtitle: "", permissionSelectable: false,
                models: [ModelOption(id: "big", label: "Big", maxRequired: true)], maxControl: true
            ),
        ])
    }

    private func form(
        _ choices: TaskOptions, agent: String = "cursor", defaults: UserDefaults = UserDefaults(suiteName: UUID().uuidString)!
    ) -> TaskComposer {
        let form = TaskComposer(macId: "mac")
        form.selectAgent(agent)
        form.applyDefaults(options: choices, defaults: defaults)
        form.text = "Do it"
        return form
    }

    func testADefaultThatNeedsMaxBlocksTheTaskUntilMaxIsAllowed() throws {
        let choices = options(), form = form(options())
        XCTAssertTrue(form.offersMaxControl(in: choices))
        XCTAssertFalse(form.allowMax, "off for every new task")
        XCTAssertEqual(form.maxRequired(in: choices), true, "with no model picked the default is what runs")
        XCTAssertTrue(form.isBlockedByMax(in: choices))
        XCTAssertFalse(form.canSubmit(options: choices))
        XCTAssertNil(form.draft(options: choices))

        form.setAllowMax(true, in: choices)
        XCTAssertFalse(form.isBlockedByMax(in: choices))
        XCTAssertTrue(form.canSubmit(options: choices))
        let draft = try XCTUnwrap(form.draft(options: choices))
        XCTAssertTrue(draft.allowMax)
    }

    func testAModelThatDoesNotNeedMaxStartsWithMaxLeftOff() throws {
        let choices = options(), form = form(options())
        form.selectModel("small", in: choices)
        XCTAssertEqual(form.maxRequired(in: choices), false)
        XCTAssertFalse(form.isBlockedByMax(in: choices))
        let draft = try XCTUnwrap(form.draft(options: choices))
        XCTAssertFalse(draft.allowMax)
        XCTAssertEqual(draft.model, "small")
        // The picked model decides, whatever the default is.
        form.selectModel("big", in: choices)
        XCTAssertTrue(form.isBlockedByMax(in: choices))
        form.selectModel(nil, in: choices)
        XCTAssertEqual(form.maxRequired(in: choices), true)
    }

    func testAnUnknownVerdictIsNeitherBlockedNorReadAsNo() throws {
        let choices = options(defaultNeedsMax: nil), form = form(options(defaultNeedsMax: nil))
        XCTAssertNil(form.maxRequired(in: choices), "no verdict for the default")
        XCTAssertFalse(form.isBlockedByMax(in: choices))
        form.selectModel("fresh", in: choices)
        XCTAssertNil(form.maxRequired(in: choices), "a model the Mac cannot tell about")
        XCTAssertFalse(form.isBlockedByMax(in: choices))
        XCTAssertNotNil(form.draft(options: choices), "an unknown verdict still starts")
    }

    func testAllowMaxIsNeverRememberedAndResetsWhenTheAgentMacOrControlChanges() {
        let choices = options(), defaults = UserDefaults(suiteName: UUID().uuidString)!
        let first = form(choices, defaults: defaults)
        first.selectModel("small", in: choices)
        first.setAllowMax(true, in: choices)
        first.remember(options: choices, defaults: defaults)
        let next = form(choices, defaults: defaults)
        XCTAssertEqual(next.modelId, "small", "the model is remembered like any other")
        XCTAssertFalse(next.allowMax)

        first.selectAgent("claude")
        XCTAssertFalse(first.allowMax)
        first.selectAgent("cursor")
        XCTAssertFalse(first.allowMax, "coming back does not bring it back")

        first.setAllowMax(true, in: choices)
        first.switchMac(to: "other")
        XCTAssertFalse(first.allowMax)

        let third = form(choices)
        third.setAllowMax(true, in: choices)
        third.applyDefaults(options: options(control: nil), defaults: defaults)
        XCTAssertFalse(third.allowMax, "a Mac that stops controlling Max leaves nothing switched on")
    }

    func testOnlyCursorOnAMacThatControlsMaxHasTheSwitch() throws {
        let old = options(control: nil), oldForm = form(old)
        XCTAssertFalse(oldForm.offersMaxControl(in: old))
        oldForm.setAllowMax(true, in: old)
        XCTAssertFalse(oldForm.allowMax)
        XCTAssertNil(oldForm.maxRequired(in: old), "an older Mac says nothing about Max")
        XCTAssertFalse(oldForm.isBlockedByMax(in: old))
        XCTAssertFalse(try XCTUnwrap(oldForm.draft(options: old)).allowMax)

        // The wire field is Cursor's alone, even from a Mac that (wrongly) marks another Agent.
        let choices = options(), claude = form(choices, agent: "claude")
        XCTAssertFalse(claude.offersMaxControl(in: choices))
        claude.setAllowMax(true, in: choices)
        XCTAssertFalse(try XCTUnwrap(claude.draft(options: choices)).allowMax)
    }

    func testTheWireCarriesAllowMaxOnlyWhenItIsSet() throws {
        let off = TaskCreate(clientMsgId: "c", workspaceId: "w", agentKind: "cursor", permission: "default", text: "Go")
        XCTAssertNil(try AHJSON.tree(off)["allowMax"], "absent, exactly what an older phone sends")
        let on = TaskCreate(clientMsgId: "c", workspaceId: "w", agentKind: "cursor", permission: "default", allowMax: true, text: "Go")
        XCTAssertEqual(try AHJSON.tree(on)["allowMax"], .bool(true))
    }

    func testTheMacsMaxMarksDecodeAndUnknownStaysAbsent() throws {
        let json = #"""
        {"kind":"cursor","label":"Cursor","ready":true,"subtitle":"cli","permissionSelectable":false,"maxControl":true,
         "defaultModel":{"label":"Big","maxRequired":true},
         "models":[{"id":"a","label":"A","maxRequired":false},{"id":"b","label":"B","maxRequired":true},{"id":"c","label":"C"}]}
        """#
        let agent = try AHJSON.decoder.decode(AgentOption.self, from: Data(json.utf8))
        XCTAssertEqual(agent.maxControl, true)
        XCTAssertEqual(agent.defaultModel?.maxRequired, true)
        XCTAssertEqual(agent.models?.map(\.maxRequired), [false, true, nil])
        // A Mac that does not know about Max sends none of it.
        let plain = try AHJSON.decoder.decode(
            AgentOption.self,
            from: Data(#"{"kind":"cursor","label":"Cursor","ready":true,"subtitle":"","permissionSelectable":false}"#.utf8)
        )
        XCTAssertNil(plain.maxControl)
    }
}
