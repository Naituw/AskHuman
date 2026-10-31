import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

@MainActor
final class AgentStoreTests: XCTestCase {
    private var backend: MockBackend!
    private var store: AppStore!

    private func make(latency: Double = 0, scenario: MockBackend.Scenario = .demo) async {
        backend = MockBackend(scenario: scenario, latency: latency)
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
        XCTAssertEqual(sections.map(\.id), ["waiting", "attention", "working", "idle"])
        XCTAssertEqual(sections[0].items.map(\.agentKey), [MockData.waitingAgentKey])
        XCTAssertEqual(Set(sections[2].items.map(\.agentKey)), [MockData.workingAgentKey, MockData.officeAgentKey])
        XCTAssertEqual(store.waitingAgentCount, 1)
    }

    func testFiltersNarrowTheAgents() async {
        await make()
        XCTAssertEqual(store.agentItems(filter: .all).count, 6)
        XCTAssertEqual(store.agentItems(filter: .mac(MockData.officeMacId)).map(\.agentKey).sorted(), [MockData.officeAgentKey, MockData.unsupportedAgentKey].sorted())
        store.agentFilter = .mac(MockData.officeMacId)
        XCTAssertEqual(store.agentItems().count, 2)
        XCTAssertEqual(store.agentSections().map(\.id), ["working", "idle"])
    }

    func testFailedIdleAgentsRemainVisibleBeforeWorkingAgents() async {
        await make()
        await backend.setAgentResult(macId: MockData.homeMacId, agentKey: MockData.workingAgentKey,
            result: AgentTurnResult(outcome: "failed", reason: "runtime_error", at: 123_000))
        await eventually { self.store.agent(id: self.working.id)?.needsAttention == true }
        let sections = store.agentSections()
        XCTAssertEqual(Array(sections.map(\.id).prefix(3)), ["waiting", "attention", "working"])
        XCTAssertEqual(Set(sections[1].items.map(\.agentKey)), [MockData.workingAgentKey, MockData.endedAgentKey])
        XCTAssertEqual(sections[1].items.first?.view.state, "idle")
    }

    func testContinuationStartsANewRoundWithoutTouchingTheDraft() async throws {
        await make()
        let old = working
        let composer = store.interjectComposer(for: old)
        composer.text = "Keep this unsent note"
        await backend.setAgentResult(macId: old.macId, agentKey: old.agentKey,
            result: AgentTurnResult(outcome: "incomplete", turnId: "old", reason: "no_progress", at: 1, id: "result-1"))
        await eventually { self.store.agent(id: old.id)?.needsAttention == true }
        let outcome = try await store.continueAgent(old)
        XCTAssertEqual(outcome.status, "delivered")
        XCTAssertEqual(outcome.notificationRegistered, true)
        XCTAssertEqual(composer.text, "Keep this unsent note")
        await eventually { self.store.agent(id: old.id)?.view.state == "working" }
        XCTAssertNil(store.agent(id: old.id)?.view.lastTurnResult)
        do { _ = try await store.continueAgent(old); XCTFail("a second click must not submit") } catch {}
        let records = await backend.interjects
        XCTAssertEqual(records.count, 1)
    }

    func testUnknownContinuationKeepsItsIdentityUntilTheOriginalChatIsChecked() async throws {
        await make()
        let item = working
        await backend.setAgentResult(macId: item.macId, agentKey: item.agentKey,
            result: AgentTurnResult(outcome: "failed", at: 1, id: "result-2"))
        await eventually { self.store.agent(id: item.id)?.needsAttention == true }
        await backend.setNextInterjectFailure("controlUnknown")
        do { _ = try await store.continueAgent(item); XCTFail("unknown") } catch {}
        let action = store.continuation(for: item)
        let id = action.messageId
        XCTAssertNotNil(id)
        XCTAssertTrue(action.needsDeliveryReview)
        do { _ = try await store.continueAgent(item); XCTFail("review is required") } catch {}
        XCTAssertEqual(action.messageId, id)
        action.confirmDeliveryReviewed()
        _ = try await store.continueAgent(item)
        XCTAssertFalse(action.needsDeliveryReview)
    }

    func testUnknownDeliveryCannotBeBypassedThroughTheOtherEntryPoint() async throws {
        await make()
        let item = working
        await backend.setAgentResult(macId: item.macId, agentKey: item.agentKey,
            result: AgentTurnResult(outcome: "failed", at: 1, id: "result-3"))
        await eventually { self.store.agent(id: item.id)?.needsAttention == true }
        let composer = store.interjectComposer(for: item)
        composer.text = "Unsent draft"
        composer.needsDeliveryReview = true
        do { _ = try await store.continueAgent(item); XCTFail("the first send needs review") } catch {}
        store.confirmAgentDeliveryReviewed(item)
        store.continuation(for: item).needsDeliveryReview = true
        do { _ = try await store.send(composer, to: item); XCTFail("the continuation needs review") } catch {}
        let records = await backend.interjects
        XCTAssertTrue(records.isEmpty)
        XCTAssertEqual(composer.text, "Unsent draft")
    }

    func testAFullNotificationListDoesNotHideSuccessfulNativeSubmission() async throws {
        await make(scenario: .agentResults)
        await backend.setFollowLimit(2)
        let item = try XCTUnwrap(store.agent(id: AgentItem.key(MockData.homeMacId, MockData.endedAgentKey)))
        let outcome = try await store.continueAgent(item)
        XCTAssertEqual(outcome.status, "delivered")
        XCTAssertEqual(outcome.notificationRegistered, false)
        XCTAssertNotNil(store.banner, "the list result row disappears when the new round starts, so the warning must outlive it")
        await eventually { self.store.agent(id: item.id)?.view.state == "working" }
    }

    // MARK: follow

    func testFollowingAndUnfollowingChangesTheList() async {
        await make()
        let waiting = store.agent(id: AgentItem.key(MockData.homeMacId, MockData.waitingAgentKey))!
        XCTAssertFalse(waiting.view.followed)
        let followed = await store.setFollow(waiting, on: true)
        XCTAssertTrue(followed)
        await eventually { self.store.agent(id: waiting.id)?.view.followed == true }
        XCTAssertEqual(store.state.agents.filter(\.view.followed).count, 4)
        await store.setFollow(waiting, on: false)
        await eventually { self.store.agent(id: waiting.id)?.view.followed == false }
    }

    func testTheFollowLimitRefusesQuietlyAndKeepsTheList() async {
        await make()
        await backend.setFollowLimit(2)
        let waiting = store.agent(id: AgentItem.key(MockData.homeMacId, MockData.waitingAgentKey))!
        let followed = await store.setFollow(waiting, on: true)
        XCTAssertFalse(followed, "the home Mac already follows two Agents")
        XCTAssertNil(store.banner, "nobody asked for a follow on the screen: nothing to tell")
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

    // MARK: notifications

    func testTappingAFollowNotificationOpensTheAgentAndARequestOneOpensTheRequest() async {
        await make()
        let agentKey = MockData.workingAgentKey
        store.openNotification(macId: MockData.homeMacId, rid: agentKey, kind: "follow.done")
        XCTAssertEqual(store.tab, .agents)
        XCTAssertEqual(store.agentPath, [AgentItem.key(MockData.homeMacId, agentKey)])

        let request = store.state.inbox.first!
        store.openNotification(macId: request.macId, rid: request.requestId, kind: "ask")
        XCTAssertEqual(store.tab, .inbox)
        XCTAssertEqual(store.inboxPath, [request.id])
    }

    func testANotificationForWhatIsOnScreenIsNotShownAgain() async {
        await make()
        let request = store.state.inbox.first!
        XCTAssertTrue(store.coversNotification(macId: request.macId, rid: request.requestId, kind: "ask"))
        XCTAssertTrue(store.coversNotification(macId: MockData.homeMacId, rid: MockData.workingAgentKey, kind: "follow.waiting"))
        XCTAssertFalse(store.coversNotification(macId: MockData.homeMacId, rid: "unknown", kind: "ask"))
        XCTAssertFalse(store.coversNotification(macId: MockData.homeMacId, rid: "claude:unknown", kind: "follow.done"))
    }

    /// The Macs the app asked the backend to check the link of, once the asynchronous call went through.
    private func linkChecks() async -> [String] {
        try? await Task.sleep(for: .milliseconds(80))
        return await backend.linkChecks
    }

    func testANotificationForARequestTheInboxLacksHasTheLinkChecked() async {
        await make()
        let request = store.state.inbox.first!
        // What the inbox holds, news that a request was handled and an Agent's news say nothing
        // about a request the phone missed.
        store.noteUnknownRequest(macId: request.macId, rid: request.requestId, kind: "ask")
        store.noteUnknownRequest(macId: MockData.homeMacId, rid: "unknown", kind: "resolved")
        store.noteUnknownRequest(macId: MockData.homeMacId, rid: "claude:unknown", kind: "follow.done")
        store.noteUnknownRequest(macId: MockData.homeMacId, rid: "unknown", kind: nil)
        let none = await linkChecks()
        XCTAssertTrue(none.isEmpty)

        store.noteUnknownRequest(macId: MockData.homeMacId, rid: "unknown", kind: "ask")
        let checked = await linkChecks()
        XCTAssertEqual(checked, [MockData.homeMacId])
    }

    func testARequestThatLeftTheInboxJustNowIsNotAMissingOne() async {
        await make()
        let request = store.state.inbox.first!
        // It ended a moment ago: the inbox lost it, but the app still knows it (its page stays open).
        await backend.simulateFinishedElsewhere(requestId: request.requestId)
        await eventually { self.store.item(id: request.id) == nil }
        XCTAssertNotNil(store.displayItem(id: request.id))
        store.noteUnknownRequest(macId: request.macId, rid: request.requestId, kind: "ask")
        let checks = await linkChecks()
        XCTAssertTrue(checks.isEmpty)
    }

    func testNoLinkIsCheckedForANotificationWhileTheAppIsInTheBackground() async {
        await make()
        await store.setForeground(false)
        await eventually { !self.store.state.foreground }
        store.noteUnknownRequest(macId: MockData.homeMacId, rid: "unknown", kind: "ask")
        let checks = await linkChecks()
        XCTAssertTrue(checks.isEmpty)
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

    func testUncertainDeliveryRetainsTheDraftAndIdentityUntilTheChatIsReviewed() async throws {
        await make()
        let item = working
        let composer = store.interjectComposer(for: item)
        composer.text = "Please continue"
        let attachment = try composer.addAttachment(name: "note.txt", mime: "text/plain", data: Data("note".utf8), isImage: false)
        for reason in ["desktopUnknown", "controlUnknown", "interrupted", "conflict"] {
            composer.confirmDeliveryReviewed()
            composer.pendingMessageId = "same-message"
            await backend.setNextInterjectFailure(reason)
            do {
                _ = try await store.send(composer, to: item)
                XCTFail("expected an uncertain delivery")
            } catch BackendError.rejected(let actual) {
                XCTAssertEqual(actual, reason)
            }
            XCTAssertEqual(composer.pendingMessageId, "same-message")
            XCTAssertTrue(composer.needsDeliveryReview)
            XCTAssertEqual(composer.text, "Please continue")
            XCTAssertEqual(composer.attachments.map(\.id), [attachment.id])
            XCTAssertFalse(composer.isSending)
        }
        composer.confirmDeliveryReviewed()
        XCTAssertNil(composer.pendingMessageId)
        XCTAssertFalse(composer.needsDeliveryReview)
        XCTAssertEqual(composer.text, "Please continue")
    }

    func testADefiniteDesktopRefusalKeepsTheDraftButAllowsAFreshAttempt() async throws {
        await make()
        let item = working
        let composer = store.interjectComposer(for: item)
        composer.text = "Continue"
        await backend.setNextInterjectFailure("desktopSendFailed")
        do {
            _ = try await store.send(composer, to: item)
            XCTFail("expected a refusal")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "desktopSendFailed")
        }
        XCTAssertNil(composer.pendingMessageId)
        XCTAssertFalse(composer.needsDeliveryReview)
        XCTAssertEqual(composer.text, "Continue")
        _ = try await store.send(composer, to: item)
        XCTAssertEqual(composer.text, "")
        XCTAssertNil(composer.pendingMessageId)
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
        let early = try await store.createTask()
        XCTAssertNil(early, "the Mac asks and the user has not chosen how the Agent may act")
        store.taskComposer?.permissionChoice = .standard
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
        store.taskComposer?.permissionChoice = .standard
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
