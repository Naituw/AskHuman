import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

@MainActor
final class AgentStoreTests: XCTestCase {
    private var backend: MockBackend!
    private var store: AppStore!

    private func make(latency: Double = 0) async {
        backend = MockBackend(scenario: .demo, latency: latency)
        let defaults = UserDefaults(suiteName: "ah-test-" + UUID().uuidString)!
        store = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        await store.start()
        await eventually("first state") { self.store.state.foreground }
    }

    private var working: AgentItem { store.agent(id: AgentItem.key(MockData.homeMacId, MockData.workingAgentKey))! }

    // MARK: list

    func testAgentsAreGroupedByStateWithQuestionsFirst() async {
        await make()
        let sections = store.agentSections()
        XCTAssertEqual(sections.map(\.id), ["waiting", "working", "idle", "ended"])
        XCTAssertEqual(sections[0].items.map(\.agentKey), [MockData.waitingAgentKey])
        XCTAssertEqual(Set(sections[1].items.map(\.agentKey)), [MockData.workingAgentKey, MockData.officeAgentKey])
        XCTAssertEqual(store.waitingAgentCount, 1)
    }

    func testFiltersNarrowTheAgents() async {
        await make()
        XCTAssertEqual(store.agentItems(filter: .all).count, 6)
        XCTAssertEqual(store.agentItems(filter: .followed).count, 3)
        XCTAssertEqual(store.agentItems(filter: .mac(MockData.officeMacId)).map(\.agentKey).sorted(), [MockData.officeAgentKey, MockData.unsupportedAgentKey].sorted())
        store.agentFilter = .followed
        XCTAssertEqual(store.agentItems().count, 3)
        XCTAssertEqual(store.agentSections().map(\.id), ["working", "idle"])
    }

    // MARK: follow

    func testFollowingAndUnfollowingChangesTheList() async {
        await make()
        let waiting = store.agent(id: AgentItem.key(MockData.homeMacId, MockData.waitingAgentKey))!
        XCTAssertFalse(waiting.view.followed)
        let followed = await store.setFollow(waiting, on: true)
        XCTAssertTrue(followed)
        await eventually { self.store.agent(id: waiting.id)?.view.followed == true }
        XCTAssertEqual(store.followedCount, 4)
        await store.setFollow(waiting, on: false)
        await eventually { self.store.agent(id: waiting.id)?.view.followed == false }
    }

    func testTheFollowLimitShowsABannerAndKeepsTheList() async {
        await make()
        await backend.setFollowLimit(2)
        let waiting = store.agent(id: AgentItem.key(MockData.homeMacId, MockData.waitingAgentKey))!
        let followed = await store.setFollow(waiting, on: true)
        XCTAssertFalse(followed, "the home Mac already follows two Agents")
        XCTAssertEqual(store.banner?.kind, .followLimit)
        XCTAssertEqual(store.agent(id: waiting.id)?.view.followed, false)
    }

    func testFollowEventsBecomeBannersUnlessThePageIsOpen() async {
        await make()
        await backend.simulateFollowEvent(kind: "done")
        await eventually { self.store.banner != nil }
        XCTAssertEqual(store.banner?.kind, .agent(macId: MockData.homeMacId, agentKey: MockData.workingAgentKey, event: "done"))
        XCTAssertEqual(store.banner?.title, "Fix flaky popup tests")
        store.dismissBanner()

        store.openAgent(macId: MockData.homeMacId, agentKey: MockData.workingAgentKey)
        XCTAssertEqual(store.tab, .agents)
        await backend.simulateFollowEvent(kind: "failed")
        try? await Task.sleep(for: .milliseconds(80))
        XCTAssertNil(store.banner, "that Agent's page shows it already")

        store.agentPath = []
        await store.setForeground(false)
        await eventually { !self.store.state.foreground }
        await backend.simulateFollowEvent(kind: "done")
        try? await Task.sleep(for: .milliseconds(80))
        XCTAssertNil(store.banner, "pushes cover the background")
    }

    // MARK: watch

    func testTheLiveFramesFollowThePageOnTheStack() async {
        await make()
        let id = working.id
        store.openAgent(macId: MockData.homeMacId, agentKey: MockData.workingAgentKey)
        XCTAssertEqual(store.agentPath, [id])
        await eventually("a frame") { self.store.watchState(id: id)?.frame != nil }
        XCTAssertEqual(store.watchState(id: id)?.frame?.phase, "working")
        // The mock keeps advancing the frames while the page is open.
        await eventually("a later frame") { (self.store.watchState(id: id)?.seq ?? 0) > 1 }

        store.agentPath = []
        await eventually("the watch is closed") { self.store.watchState(id: id) == nil }
    }

    func testAnEndedAgentStaysOnItsPageAfterItLeavesTheList() async {
        await make()
        let id = working.id
        store.openAgent(macId: MockData.homeMacId, agentKey: MockData.workingAgentKey)
        await backend.dropAgent(macId: MockData.homeMacId, agentKey: MockData.workingAgentKey)
        await eventually { self.store.agent(id: id) == nil }
        XCTAssertEqual(store.displayAgent(id: id)?.agentKey, MockData.workingAgentKey, "the page keeps the last known version")
    }

    // MARK: interjections

    func testAMessageToAWorkingAgentIsQueuedUntilItIsRead() async throws {
        await make()
        await backend.setInterjectConsumeDelay(nil)
        let item = working
        let outcome = try await store.interject(item, text: "Please also add a test", files: [], clientMsgId: nil, progress: { _ in })
        XCTAssertEqual(outcome, InterjectOutcome(status: "queued", pending: 1))
        await eventually { self.store.agent(id: item.id)?.view.pendingInterject == 1 }

        await backend.consumeInterjects(macId: item.macId, agentKey: item.agentKey)
        await eventually { self.store.agent(id: item.id)?.view.pendingInterject == 0 }
        await eventually("the notice") { self.store.consumed[item.id] != nil }
    }

    func testAMessageToAnAgentThatIsNotWorkingIsRefused() async throws {
        await make()
        let idle = store.agent(id: AgentItem.key(MockData.homeMacId, MockData.idleAgentKey))!
        do {
            _ = try await store.interject(idle, text: "Hello", files: [], clientMsgId: nil, progress: { _ in })
            XCTFail("an idle Agent does not read messages")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "notWorking")
        }
        let unsupported = store.agent(id: AgentItem.key(MockData.officeMacId, MockData.unsupportedAgentKey))!
        do {
            _ = try await store.interject(unsupported, text: "Hello", files: [], clientMsgId: nil, progress: { _ in })
            XCTFail("Codex App sessions cannot be reached")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "unsupported")
        }
    }

    func testRecallingTakesTheTextBack() async throws {
        await make()
        await backend.setInterjectConsumeDelay(nil)
        let item = working
        _ = try await store.interject(item, text: "First", files: [], clientMsgId: nil, progress: { _ in })
        _ = try await store.interject(item, text: "Second", files: [], clientMsgId: nil, progress: { _ in })
        await eventually { self.store.agent(id: item.id)?.view.pendingInterject == 2 }

        let text = try await store.recallInterject(item)
        XCTAssertEqual(text, "First\n\nSecond")
        await eventually { self.store.agent(id: item.id)?.view.pendingInterject == 0 }
    }

    // MARK: new tasks

    func testTheNewTaskFormStartsOnAMacThatCanRunTasks() async {
        await make()
        XCTAssertEqual(store.taskMacs.map(\.id).sorted(), [MockData.homeMacId, MockData.officeMacId].sorted())
        store.presentNewTask(macId: MockData.homeMacId)
        let composer = try? XCTUnwrap(store.taskComposer)
        XCTAssertEqual(composer?.macId, MockData.homeMacId)
        await eventually("the choices") {
            if case .ready? = self.store.state.taskOptions[MockData.homeMacId] { return composer?.workspaceId != nil }
            return false
        }
        XCTAssertEqual(composer?.workspaceId, "ws-humaninloop", "the pinned workspace")
        XCTAssertEqual(composer?.agentKind, "claude", "the first Agent that is ready")
        await eventually("the TODOs of the workspace") { self.store.state.todos(macId: MockData.homeMacId, workspaceId: "ws-humaninloop") != nil }
    }

    func testAMacThatCannotRunTasksExplainsWhy() async {
        await make()
        store.presentNewTask(macId: MockData.officeMacId)
        await eventually("the choices") {
            if case .ready? = self.store.state.taskOptions[MockData.officeMacId] { return true }
            return false
        }
        guard case .ready(let options)? = store.state.taskOptions[MockData.officeMacId] else { return XCTFail("no options") }
        XCTAssertFalse(options.enabled)
        XCTAssertEqual(options.blocked, "noTerminal")
        XCTAssertNil(store.taskComposer?.draft(options: options))
    }

    func testSwitchingTheMacStartsOverWithItsOwnChoices() async {
        await make()
        store.presentNewTask(macId: MockData.homeMacId)
        await eventually { self.store.taskComposer?.workspaceId != nil }
        store.taskComposer?.text = "Keep my words"
        store.selectTaskMac(MockData.officeMacId)
        XCTAssertEqual(store.taskComposer?.macId, MockData.officeMacId)
        XCTAssertNil(store.taskComposer?.workspaceId)
        XCTAssertEqual(store.taskComposer?.text, "Keep my words", "what the user typed stays")
    }

    func testANewTaskBecomesAFollowedAgent() async throws {
        await make()
        store.presentNewTask(macId: MockData.homeMacId)
        await eventually { self.store.taskComposer?.workspaceId != nil }
        store.taskComposer?.text = "Add a dark mode toggle"
        let id = try await store.createTask()
        let taskId = try XCTUnwrap(id)
        await eventually("watching") { self.store.task(id: taskId)?.stage == "watching" }
        let task = try XCTUnwrap(store.task(id: taskId))
        XCTAssertTrue(task.foundAgent)
        let agent = try XCTUnwrap(store.agent(id: AgentItem.key(MockData.homeMacId, try XCTUnwrap(task.agentKey))))
        XCTAssertTrue(agent.view.followed)
        XCTAssertEqual(agent.view.state, "working")
        let drafts = await backend.createdTasks
        XCTAssertEqual(drafts.first?.workspaceName, "HumanInLoop")
        XCTAssertEqual(drafts.first?.permission, "default")
        XCTAssertEqual(drafts.first?.text, "Add a dark mode toggle")
    }

    func testAFailedTaskShowsTheReason() async throws {
        await make()
        await backend.setFailNextTask("terminalFailed")
        store.presentNewTask(macId: MockData.homeMacId)
        await eventually { self.store.taskComposer?.workspaceId != nil }
        store.taskComposer?.text = "Do it"
        let created = try await store.createTask()
        let taskId = try XCTUnwrap(created)
        await eventually("failed") { self.store.task(id: taskId)?.stage == "failed" }
        XCTAssertEqual(store.task(id: taskId)?.reason, "terminalFailed")
        XCTAssertEqual(store.task(id: taskId)?.canRetry, false)
        await store.dismissTask(taskId)
        await eventually("the task is gone") { self.store.task(id: taskId) == nil }
    }

    func testAnIncompleteFormSendsNothing() async throws {
        await make()
        store.presentNewTask(macId: MockData.homeMacId)
        await eventually { self.store.taskComposer?.workspaceId != nil }
        let id = try await store.createTask()
        XCTAssertNil(id, "no text and no TODO")
        let created = await backend.createdTasks
        XCTAssertTrue(created.isEmpty)
    }

    // MARK: slot

    func testTakingOverQuestionsMarksTheMacAsActive() async throws {
        await make()
        let home = try XCTUnwrap(store.macs.first { $0.id == MockData.homeMacId })
        XCTAssertEqual(home.slot?.active, false)
        try await store.takeSlot(home)
        await eventually { self.store.macs.first { $0.id == MockData.homeMacId }?.slot?.active == true }
    }
}
