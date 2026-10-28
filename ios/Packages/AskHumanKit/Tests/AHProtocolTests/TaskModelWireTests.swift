import XCTest
@testable import AHProtocol

/// The model and effort fields of `task.options` / `task.create` (`docs/specs/ios-task-model.md`):
/// optional on both sides, so a Mac and an app of different ages still understand each other.
final class TaskModelWireTests: XCTestCase {
    private func agent(_ json: String) throws -> AgentOption {
        try AHJSON.decoder.decode(AgentOption.self, from: Data(json.utf8))
    }

    private let base = #""kind": "codex", "label": "Codex", "ready": true, "subtitle": "codex", "permissionSelectable": true"#

    func testATaskWithoutAPickLooksExactlyAsItDidBefore() throws {
        let plain = TaskCreate(clientMsgId: "c", workspaceId: "w", agentKind: "claude", permission: "default", text: "Go")
        XCTAssertEqual(
            try AHJSON.tree(plain),
            .object([
                "clientMsgId": .string("c"), "workspaceId": .string("w"), "agentKind": .string("claude"),
                "permission": .string("default"), "text": .string("Go"),
            ]),
            "an older Mac must see no new key (and so computes the same idempotency hash)"
        )
    }

    func testThePickIsAddedOnlyWhenThereIsOne() throws {
        let both = TaskCreate(clientMsgId: "c", workspaceId: "w", agentKind: "codex", permission: "default", model: "model-a", effort: "high", text: "Go")
        let tree = try AHJSON.tree(both)
        XCTAssertEqual(tree["model"], .string("model-a"))
        XCTAssertEqual(tree["effort"], .string("high"))

        let effortOnly = TaskCreate(clientMsgId: "c", workspaceId: "w", agentKind: "claude", permission: "default", effort: "max", text: "Go")
        let alone = try AHJSON.tree(effortOnly)
        XCTAssertNil(alone["model"], "Default sends no model")
        XCTAssertEqual(alone["effort"], .string("max"))
    }

    func testTheModelsOfAnAgentRoundTrip() throws {
        let text = """
        {\(base), "models": [
          {"id": "model-a", "label": "Model A", "note": "A note", "efforts": ["low", "high"], "defaultEffort": "low"},
          {"id": "model-b", "label": "Model B"}
        ], "defaultModel": {"label": "Model A", "efforts": ["low", "high"], "defaultEffort": "high"}}
        """
        let option = try agent(text)
        XCTAssertEqual(option.models?.map(\.id), ["model-a", "model-b"])
        XCTAssertEqual(option.models?[0].note, "A note")
        XCTAssertEqual(option.models?[0].efforts, ["low", "high"])
        XCTAssertEqual(option.models?[0].defaultEffort, "low")
        XCTAssertNil(option.models?[1].efforts)
        XCTAssertEqual(option.defaultModel, DefaultModel(label: "Model A", efforts: ["low", "high"], defaultEffort: "high"))

        let again = try AHJSON.tree(option)
        let original = try AHJSON.decoder.decode(JSONValue.self, from: Data(text.utf8))
        XCTAssertEqual(again, original)
    }

    func testAnOlderMacSendsNoModelsAndThatIsFine() throws {
        let option = try agent("{\(base)}")
        XCTAssertNil(option.models)
        XCTAssertNil(option.defaultModel)
        let tree = try AHJSON.tree(option)
        XCTAssertNil(tree["models"], "absent stays absent")
        XCTAssertNil(tree["defaultModel"])
    }

    func testAModelEntryThatDoesNotDecodeIsSkippedAndTheRestStays() throws {
        let option = try agent("""
        {\(base), "models": [
          {"id": "model-a", "label": "Model A"},
          {"id": 5, "label": "number"},
          {"label": "no id"},
          "just a string",
          null,
          {"id": "model-b", "label": "Model B", "efforts": ["low", 3]},
          {"id": "model-c", "label": "Model C", "efforts": ["low"]}
        ]}
        """)
        XCTAssertEqual(option.models?.map(\.id), ["model-a", "model-c"])
        XCTAssertTrue(option.ready, "the rest of the Agent is untouched")
    }

    func testAListOrDefaultThatIsNotTheRightShapeNeverFailsTheOptions() throws {
        let wrongList = try agent(#"{\#(base), "models": "oops", "defaultModel": {"label": "Model A"}}"#)
        XCTAssertNil(wrongList.models)
        XCTAssertEqual(wrongList.defaultModel?.label, "Model A")

        let wrongDefault = try agent(#"{\#(base), "models": [{"id": "model-a", "label": "Model A"}], "defaultModel": 7}"#)
        XCTAssertEqual(wrongDefault.models?.map(\.id), ["model-a"])
        XCTAssertNil(wrongDefault.defaultModel)
    }

    func testAMissingRequiredFieldOfTheAgentStillFails() {
        XCTAssertThrowsError(try agent(#"{"kind": "codex", "label": "Codex"}"#), "only the model fields are lenient")
    }

    func testTheWholeOptionsMessageSurvivesABadModel() throws {
        let body = try AHJSON.decoder.decode(JSONValue.self, from: Data("""
        {"enabled": true, "permissionMode": "ask", "workspaces": [],
         "agents": [{\(base), "models": [{"id": "model-a"}, {"id": "model-b", "label": "Model B"}]}]}
        """.utf8))
        let options = try AHJSON.value(TaskOptions.self, from: body)
        XCTAssertEqual(options.agents.first?.models?.map(\.id), ["model-b"])
    }
}
