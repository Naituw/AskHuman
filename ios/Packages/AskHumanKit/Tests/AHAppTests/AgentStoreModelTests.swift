import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

/// The model and effort of the new task form over the mock backend: what the demo Mac offers, how the
/// form walks through it and what the started Agent shows (`docs/specs/ios-task-model.md`).
@MainActor
final class AgentStoreModelTests: XCTestCase {
    private var backend: MockBackend!
    private var store: AppStore!

    private func make() async {
        backend = MockBackend(scenario: .demo, latency: 0)
        let defaults = UserDefaults(suiteName: "ah-test-" + UUID().uuidString)!
        store = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        await store.start()
        await eventually("first state") { self.store.state.foreground }
    }

    /// Opens the form on the home Mac and waits for its choices.
    private func openForm() async throws -> (TaskComposer, TaskOptions) {
        store.presentNewTask(macId: MockData.homeMacId)
        let composer = try XCTUnwrap(store.taskComposer)
        await eventually("the choices") {
            if case .ready? = self.store.state.taskOptions[MockData.homeMacId] { return composer.workspaceId != nil }
            return false
        }
        guard case .ready(let options)? = store.state.taskOptions[MockData.homeMacId] else {
            XCTFail("no options")
            throw CancellationError()
        }
        return (composer, options)
    }

    func testTheDemoMacOffersModelsForTheAgentsThatCanStart() async throws {
        await make()
        let (_, options) = try await openForm()
        let byKind = Dictionary(uniqueKeysWithValues: options.agents.map { ($0.kind, $0) })
        XCTAssertEqual(byKind["claude"]?.models?.map(\.id), ["opus", "sonnet", "haiku"])
        XCTAssertGreaterThan(byKind["cursor"]?.models?.count ?? 0, 12, "long enough for the search field")
        XCTAssertNil(byKind["cursor"]?.models?.first?.efforts, "Cursor has no effort row")
        XCTAssertEqual(byKind["codex"]?.models?.first?.defaultEffort, "medium")
        XCTAssertNil(byKind["grok"]?.models, "an Agent that is not ready lists none")
    }

    func testAnOlderMacListsNoModelsAndTheFormStillWorks() async throws {
        await make()
        await backend.setOffersModels(false)
        let (composer, options) = try await openForm()
        XCTAssertTrue(options.agents.allSatisfy { $0.models == nil && $0.defaultModel == nil })
        XCTAssertFalse(composer.offersModelChoice(in: options))
        XCTAssertNil(composer.effortChoices(in: options))

        composer.text = "Add a dark mode toggle"
        composer.permissionChoice = .standard
        let id = try await store.createTask()
        XCTAssertNotNil(id)
        let drafts = await backend.createdTasks
        XCTAssertNil(drafts.first?.model)
        XCTAssertNil(drafts.first?.effort)
    }

    func testThePickReachesTheMacAndTheProgressPage() async throws {
        await make()
        let (composer, options) = try await openForm()
        composer.selectAgent("codex")
        composer.selectModel("gpt-5.1-codex-mini", in: options)
        composer.effortId = "high"
        composer.text = "Tidy the docs"
        composer.permissionChoice = .standard

        let created = try await store.createTask()
        let id = try XCTUnwrap(created)
        let drafts = await backend.createdTasks
        XCTAssertEqual(drafts.first?.model, "gpt-5.1-codex-mini")
        XCTAssertEqual(drafts.first?.effort, "high")
        XCTAssertEqual(drafts.first?.modelLabel, "GPT-5.1-Codex-Mini")
        let task = try XCTUnwrap(store.task(id: id))
        XCTAssertEqual(task.modelLabel, "GPT-5.1-Codex-Mini")
        XCTAssertEqual(task.effort, "high")
    }

    func testSubmittedChoicesReturnWhenReopeningSwitchingAgentsAndSwitchingMacs() async throws {
        await make()
        let (composer, options) = try await openForm()
        store.selectTaskAgent("codex")
        composer.selectModel("gpt-5.1-codex-mini", in: options)
        composer.effortId = "high"
        composer.text = "Tidy the docs"
        composer.permissionChoice = .standard
        let id = try await store.createTask()
        XCTAssertNotNil(id)
        store.dismissNewTask()

        let (reopened, _) = try await openForm()
        XCTAssertEqual(reopened.agentKind, "codex")
        XCTAssertEqual(reopened.modelId, "gpt-5.1-codex-mini")
        XCTAssertEqual(reopened.effortId, "high")
        XCTAssertNil(reopened.permissionChoice, "permission still requires a fresh choice")
        store.selectTaskAgent("claude")
        XCTAssertNil(reopened.modelId, "preferences are per Agent")
        store.selectTaskAgent("codex")
        XCTAssertEqual(reopened.modelId, "gpt-5.1-codex-mini")

        store.selectTaskMac(MockData.officeMacId)
        await eventually("office Mac choices") {
            if case .ready? = self.store.state.taskOptions[MockData.officeMacId] { return true }
            return false
        }
        XCTAssertNil(reopened.modelId, "the demo office Mac cannot start tasks")
        store.selectTaskMac(MockData.homeMacId)
        await eventually("home Mac choices") { reopened.workspaceId != nil }
        XCTAssertEqual(reopened.modelId, "gpt-5.1-codex-mini", "switching Macs restores the saved choice")
        XCTAssertEqual(reopened.effortId, "high")
    }

    func testCancellingAnEditKeepsTheSubmittedPreferenceAndAFailedStartStillSaves() async throws {
        await make()
        await backend.setFailNextTask("modelUnavailable")
        let (composer, options) = try await openForm()
        composer.selectModel("sonnet", in: options)
        composer.effortId = "max"
        composer.text = "Fix the build"
        composer.permissionChoice = .standard
        let created = try await store.createTask()
        let id = try XCTUnwrap(created)
        await eventually("failed start") { self.store.task(id: id)?.stage == "failed" }
        store.dismissNewTask()

        let (editing, current) = try await openForm()
        XCTAssertEqual(editing.modelId, "sonnet", "even a failed start records the choice")
        XCTAssertEqual(editing.effortId, "max")
        editing.selectModel("haiku", in: current)
        editing.effortId = "low"
        store.dismissNewTask()
        let (reopened, _) = try await openForm()
        XCTAssertEqual(reopened.modelId, "sonnet", "cancelling does not update the preference")
        XCTAssertEqual(reopened.effortId, "max")
    }

    func testTheNewAgentRunsWithWhatWasPicked() async throws {
        await make()
        let (composer, options) = try await openForm()
        composer.selectModel("sonnet", in: options)
        composer.effortId = "max"
        composer.text = "Fix the build"
        composer.permissionChoice = .standard
        let created = try await store.createTask()
        let id = try XCTUnwrap(created)
        await eventually("watching") { self.store.task(id: id)?.stage == "watching" }
        let task = try XCTUnwrap(store.task(id: id))
        let agentId = AgentItem.key(MockData.homeMacId, try XCTUnwrap(task.agentKey))

        await backend.watch(macId: MockData.homeMacId, agentKey: try XCTUnwrap(task.agentKey), open: true)
        await eventually("a frame with the pick") { self.store.watchState(id: agentId)?.frame?.model == "Sonnet" }
        XCTAssertEqual(store.watchState(id: agentId)?.frame?.reasoning, "max")
    }

    func testANewAgentThatNeededNoPickReportsNoModel() async throws {
        await make()
        let (composer, _) = try await openForm()
        composer.text = "Fix the build"
        composer.permissionChoice = .standard
        let created = try await store.createTask()
        let id = try XCTUnwrap(created)
        await eventually("watching") { self.store.task(id: id)?.stage == "watching" }
        let key = try XCTUnwrap(store.task(id: id)?.agentKey)
        await backend.watch(macId: MockData.homeMacId, agentKey: key, open: true)
        await eventually("a frame") { self.store.watchState(id: AgentItem.key(MockData.homeMacId, key))?.frame != nil }
        XCTAssertNil(store.watchState(id: AgentItem.key(MockData.homeMacId, key))?.frame?.model)
    }

    func testTheDemoAgentsReportTheirModelsInTheLiveView() async throws {
        await make()
        let working = AgentItem.key(MockData.homeMacId, MockData.workingAgentKey)
        let waiting = AgentItem.key(MockData.homeMacId, MockData.waitingAgentKey)
        let ended = AgentItem.key(MockData.homeMacId, MockData.endedAgentKey)
        for id in [working, waiting, ended] {
            let (mac, key) = try XCTUnwrap(AgentItem.split(id))
            await backend.watch(macId: mac, agentKey: key, open: true)
        }
        await eventually("the frames") {
            [working, waiting, ended].allSatisfy { self.store.watchState(id: $0)?.frame != nil }
        }
        XCTAssertEqual(store.watchState(id: working)?.frame?.model, "claude-opus-4-1")
        XCTAssertEqual(store.watchState(id: working)?.frame?.reasoning, "high")
        XCTAssertEqual(store.watchState(id: waiting)?.frame?.model, "composer-1")
        XCTAssertNil(store.watchState(id: waiting)?.frame?.reasoning, "Cursor has no effort")
        XCTAssertNil(store.watchState(id: ended)?.frame?.model, "nothing is known of that session")
    }
}
