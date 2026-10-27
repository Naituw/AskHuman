import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The model and the effort of a new task through the real engine and a fake Mac
/// (`docs/specs/ios-task-model.md`).
final class AgentModelTests: XCTestCase {
    private func offered() -> TaskOptions {
        TaskOptions(
            enabled: true, permissionMode: "ask",
            workspaces: [WorkspaceOption(id: "ws1", label: "HumanInLoop", subtitle: "~/Developer", pinned: true, lastUsedAt: 0)],
            agents: [
                AgentOption(
                    kind: "codex", label: "Codex", ready: true, subtitle: "Terminal", permissionSelectable: true,
                    models: [
                        ModelOption(id: "model-a", label: "Model A", note: "A note", efforts: ["low", "medium", "high"], defaultEffort: "medium"),
                        ModelOption(id: "model-b", label: "Model B"),
                    ],
                    defaultModel: DefaultModel(label: "Model A", efforts: ["low", "medium", "high"], defaultEffort: "high")
                ),
                AgentOption(kind: "claude", label: "Claude Code", ready: true, subtitle: "Terminal", permissionSelectable: true),
            ]
        )
    }

    private func draft(model: String? = nil, effort: String? = nil, label: String? = nil) -> TaskDraft {
        TaskDraft(
            workspaceId: "ws1", workspaceName: "HumanInLoop", agentKind: "codex", agentLabel: "Codex", permission: "default",
            model: model, effort: effort, modelLabel: label, text: "Fix the build"
        )
    }

    func testTheModelsOfTheMacArriveWithTheChoices() async throws {
        let h = Harness()
        h.mac.taskOptions = offered()
        await h.pair()
        await h.engine.loadTaskOptions(macId: h.mac.macId)
        await eventually("options") {
            if case .ready? = await h.state().taskOptions[h.mac.macId] { return true }
            return false
        }
        guard case .ready(let options)? = await h.state().taskOptions[h.mac.macId] else { return XCTFail("no options") }
        let codex = try unwrap(options.agents.first { $0.kind == "codex" })
        XCTAssertEqual(codex.models?.map(\.id), ["model-a", "model-b"])
        XCTAssertEqual(codex.models?.first?.efforts, ["low", "medium", "high"])
        XCTAssertEqual(codex.models?.first?.defaultEffort, "medium")
        XCTAssertEqual(codex.defaultModel?.label, "Model A")
        XCTAssertEqual(codex.defaultModel?.defaultEffort, "high")
        XCTAssertNil(options.agents.first { $0.kind == "claude" }?.models, "an Agent the Mac lists no models for has none")
    }

    func testThePickTravelsInTheTaskAndStaysOnTheProgress() async throws {
        let h = Harness()
        h.mac.taskScript = [("accepted", nil, nil), ("launching", nil, nil)]
        await h.pair()
        let id = try await h.engine.createTask(macId: h.mac.macId, draft: draft(model: "model-a", effort: "high", label: "Model A"))
        await eventually("launching") { await h.state().tasks.first?.stage == "launching" }

        let create = try h.mac.received(TaskCreate.type)[0].body(as: TaskCreate.self)
        XCTAssertEqual(create.clientMsgId, id)
        XCTAssertEqual(create.model, "model-a")
        XCTAssertEqual(create.effort, "high")
        let task = try unwrap(await h.state().tasks.first)
        XCTAssertEqual(task.modelLabel, "Model A", "the progress page shows what was picked, by name")
        XCTAssertEqual(task.effort, "high")
        XCTAssertTrue(task.hasModelPick)
    }

    func testATaskWithoutAPickSendsNoModelKeysAndShowsNoPick() async throws {
        let h = Harness()
        await h.pair()
        _ = try await h.engine.createTask(macId: h.mac.macId, draft: draft())
        await eventually("task.create") { h.mac.count(TaskCreate.type) == 1 }
        let plain = try h.mac.received(TaskCreate.type)[0]
        XCTAssertNil(plain.b["model"], "Default sends nothing, so an older Mac sees what it always saw")
        XCTAssertNil(plain.b["effort"])
        let task = try unwrap(await h.state().tasks.first)
        XCTAssertFalse(task.hasModelPick)
    }

    func testAnEffortWithoutAModelIsSentAlone() async throws {
        let h = Harness()
        await h.pair()
        _ = try await h.engine.createTask(macId: h.mac.macId, draft: draft(effort: "max"))
        await eventually("task.create") { h.mac.count(TaskCreate.type) == 1 }
        let plain = try h.mac.received(TaskCreate.type)[0]
        XCTAssertNil(plain.b["model"])
        XCTAssertEqual(plain.b["effort"], .string("max"))
        let task = try unwrap(await h.state().tasks.first)
        XCTAssertNil(task.modelLabel)
        XCTAssertTrue(task.hasModelPick)
    }

    func testARefusalForTheModelIsKeptAsItCameAndIsFinal() async throws {
        for reason in ["modelUnavailable", "effortUnavailable"] {
            let h = Harness()
            h.mac.taskScript = [("accepted", nil, nil), ("failed", reason, nil)]
            await h.pair()
            _ = try await h.engine.createTask(macId: h.mac.macId, draft: draft(model: "model-a", effort: "high", label: "Model A"))
            await eventually("failed") { await h.state().tasks.first?.stage == "failed" }
            let task = try unwrap(await h.state().tasks.first)
            XCTAssertEqual(task.reason, reason)
            XCTAssertFalse(task.canRetry, "the Mac said no: the form is sent again as a new task")
        }
    }

    func testARetryOfAPickedTaskSendsTheSamePickUnderTheSameId() async throws {
        let h = Harness()
        h.mac.ignoreTasks = true
        await h.pair()
        let id = try await h.engine.createTask(macId: h.mac.macId, draft: draft(model: "model-b", effort: nil, label: "Model B"))
        await eventually("task.create") { h.mac.count(TaskCreate.type) == 1 }
        await h.advance(until: { await h.state().tasks.first?.stage == "failed" }, "the accept timeout")

        h.mac.ignoreTasks = false
        try await h.engine.retryTask(id: id)
        await eventually("launched") { await h.state().tasks.first?.stage == "launched" }
        let sent = try h.mac.received(TaskCreate.type).map { try $0.body(as: TaskCreate.self) }
        XCTAssertEqual(sent.map(\.clientMsgId), [id, id])
        XCTAssertEqual(sent.map(\.model), ["model-b", "model-b"], "the Mac matches a repeat by its content: it must not differ")
        XCTAssertEqual(sent.map(\.effort), [nil, nil])
    }

    // MARK: what the session runs with

    func testTheLiveFrameCarriesTheModelAndTheEffortWhenTheMacKnowsThem() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }
        let id = AgentItem.key(h.mac.macId, "claude:s1")
        await h.engine.watch(macId: h.mac.macId, agentKey: "claude:s1", open: true)
        await eventually("the first frame") { await h.state().watches[id]?.frame != nil }
        var frame = try unwrap(await h.state().watches[id]?.frame)
        XCTAssertNil(frame.model, "a Mac that does not say shows nothing")
        XCTAssertNil(frame.reasoning)

        await h.mac.sendWatchFrame(agentKey: "claude:s1", seq: 2, text: "Second", model: "model-b", reasoning: "high")
        await eventually("the model") { await h.state().watches[id]?.frame?.model == "model-b" }
        frame = try unwrap(await h.state().watches[id]?.frame)
        XCTAssertEqual(frame.reasoning, "high")

        await h.mac.sendWatchFrame(agentKey: "claude:s1", seq: 3, text: "Third", model: "model-b")
        await eventually("the third frame") { await h.state().watches[id]?.frame?.text == "Third" }
        frame = try unwrap(await h.state().watches[id]?.frame)
        XCTAssertEqual(frame.model, "model-b")
        XCTAssertNil(frame.reasoning)
    }

    func testAnEmptyModelInAFrameCountsAsNone() throws {
        let frame = try XCTUnwrap(WatchFrameData(json: .object(["phase": .string("working"), "model": .string(""), "reasoning": .int(3)])))
        XCTAssertNil(frame.model)
        XCTAssertNil(frame.reasoning, "a value that is not text is ignored, not an error")
    }
}
