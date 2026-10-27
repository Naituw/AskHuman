import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// Agents, follow, live watch, messages to a working Agent, new tasks and the question slot, against
/// the real gateway. Sessions are made up through the hidden `__agent-hook` command, as a real
/// Agent's hooks would report them. No terminal is ever launched: the throwaway Mac keeps "Agent
/// tasks" switched off.
final class AgentE2ETests: E2ECase {
    private lazy var project: URL = {
        let url = mac.home.appendingPathComponent("project")
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }()

    // MARK: helpers

    @discardableResult
    private func hook(_ event: String, session: String, agent: String = "claude", extra: [String: Any] = [:]) async throws -> CmdResult {
        var payload: [String: Any] = ["session_id": session, "cwd": project.path]
        payload.merge(extra) { _, new in new }
        let data = try JSONSerialization.data(withJSONObject: payload)
        let result = try await mac.cli(["__agent-hook", agent, event], stdin: String(decoding: data, as: UTF8.self), timeout: 30)
        XCTAssertEqual(result.status, 0, result.stderr)
        return result
    }

    private func toolStep(_ command: String = "ls -la") -> [String: Any] {
        ["hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": ["command": command]]
    }

    private func key(_ session: String, agent: String = "claude") -> String { "\(agent):\(session)" }

    private func agent(_ session: String) async -> AgentItem? {
        await phone.state().agents.first { $0.agentKey == key(session) }
    }

    private func macId() async throws -> String { try unwrap(await phone.state().macs.first?.id) }

    /// Starts a session that is working on the Mac and shows up on the phone.
    @discardableResult
    private func startWorking(_ session: String) async throws -> AgentItem {
        try await hook("session-start", session: session)
        try await hook("turn-start", session: session)
        await eventually("\(session) working on the phone") { await self.agent(session)?.view.state == "working" }
        return try unwrap(await agent(session))
    }

    // MARK: listing

    func testSessionsOfTheMacAreListedWithTheirState() async throws {
        try await pair()
        try await hook("session-start", session: "e2e-idle")
        await eventually("an idle session") { await self.agent("e2e-idle")?.view.state == "idle" }
        try await startWorking("e2e-busy")

        let idle = try unwrap(await agent("e2e-idle"))
        XCTAssertEqual(idle.view.kind, "claude")
        XCTAssertEqual(idle.view.canInterject.ok, false)
        XCTAssertEqual(idle.view.canInterject.reason, "notWorking")
        let busy = try unwrap(await agent("e2e-busy"))
        XCTAssertTrue(busy.view.canInterject.ok)
        let macId = try await macId()
        XCTAssertEqual(busy.macId, macId)

        try await hook("session-end", session: "e2e-idle")
        await eventually("the ended session") { await self.agent("e2e-idle")?.view.state == "ended" }
        let ended = await agent("e2e-idle")
        XCTAssertEqual(ended?.view.canInterject.reason, "ended")
    }

    // MARK: live watch

    func testTheLiveViewFollowsTheSessionAndStopsWhenClosed() async throws {
        try await pair()
        let item = try await startWorking("e2e-live")
        let id = item.id

        await phone.engine.watch(macId: item.macId, agentKey: item.agentKey, open: true)
        await eventually("the first frame") { await self.phone.state().watches[id]?.frame != nil }
        let first = await phone.state().watches[id]?.frame
        XCTAssertEqual(first?.phase, "working")

        try await hook("activity", session: "e2e-live", extra: toolStep("cargo test"))
        await eventually("the tool in the list") { await self.agent("e2e-live")?.view.currentTool != nil }
        let running = await agent("e2e-live")
        XCTAssertTrue(try unwrap(running?.view.currentTool).contains("cargo test"))

        try await hook("turn-end", session: "e2e-live")
        await eventually("an idle frame") { await self.phone.state().watches[id]?.frame?.phase == "idle" }

        await phone.engine.watch(macId: item.macId, agentKey: item.agentKey, open: false)
        await eventually("the watch to be dropped") { await self.phone.state().watches[id] == nil }
        try await hook("turn-start", session: "e2e-live")
        await eventually("the list to move on") { await self.agent("e2e-live")?.view.state == "working" }
        try await Task.sleep(nanoseconds: 1_500_000_000)
        let late = await phone.state().watches[id]
        XCTAssertNil(late, "a closed watch gets no more frames")
    }

    func testWatchingASessionTheMacDoesNotKnowIsClosedAtOnce() async throws {
        try await pair()
        let macId = try await macId()
        await phone.engine.watch(macId: macId, agentKey: "claude:nobody", open: true)
        await eventually("the watch to be closed") {
            await self.phone.state().watches[AgentItem.key(macId, "claude:nobody")]?.closed == "notFound"
        }
    }

    // MARK: follow

    func testAFollowedSessionReportsWhenItsTurnEnds() async throws {
        try await pair()
        let item = try await startWorking("e2e-follow")
        try await phone.engine.setFollow(macId: item.macId, agentKey: item.agentKey, on: true)
        await eventually("the follow mark") { await self.agent("e2e-follow")?.view.followed == true }

        try await hook("turn-end", session: "e2e-follow")
        await eventually("the done event") {
            self.phone.events.contains {
                if case .followEvent(_, let event) = $0 { return event.agentKey == item.agentKey && event.kind == "done" }
                return false
            }
        }

        try await phone.engine.setFollow(macId: item.macId, agentKey: item.agentKey, on: false)
        await eventually("the mark to go") { await self.agent("e2e-follow")?.view.followed == false }
    }

    func testFollowingStopsAtTheLimit() async throws {
        try await pair()
        let macId = try await macId()
        for number in 0...Wire.maxFollow {
            try await hook("session-start", session: "e2e-many-\(number)")
        }
        await eventually("all sessions") { await self.phone.state().agents.count == Wire.maxFollow + 1 }
        for number in 0..<Wire.maxFollow {
            try await phone.engine.setFollow(macId: macId, agentKey: key("e2e-many-\(number)"), on: true)
            // The phone shows a follow at once and takes the Mac's list as the truth when it arrives:
            // let the Mac have its say before the next one, or a late list undoes the marks in between.
            await eventually("the Mac to confirm follow \(number + 1)") { await self.phone.engine.followRequests[macId] == nil }
        }
        do {
            try await phone.engine.setFollow(macId: macId, agentKey: key("e2e-many-\(Wire.maxFollow)"), on: true)
            XCTFail("the Mac should refuse a sixth follow")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "limit")
        }
        let followed = await phone.state().agents.filter(\.view.followed).count
        XCTAssertEqual(followed, Wire.maxFollow)
    }

    // MARK: messages to an Agent

    func testAMessageIsQueuedUntilTheAgentsNextStepPicksItUp() async throws {
        try await pair()
        let item = try await startWorking("e2e-msg")

        let outcome = try await phone.engine.sendInterject(
            macId: item.macId, agentKey: item.agentKey, text: "Also run the Linux build.", files: [], clientMsgId: nil, progress: { _ in }
        )
        XCTAssertEqual(outcome.status, "queued")
        XCTAssertEqual(outcome.pending, 1)
        await eventually("the queue on the list") { await self.agent("e2e-msg")?.view.pendingInterject == 1 }

        // The Agent's next tool call asks the Mac whether anything is waiting: the answer is the message.
        let step = try await hook("activity", session: "e2e-msg", extra: toolStep())
        XCTAssertTrue(step.stdout.contains("Also run the Linux build."), "hook output: \(step.stdout)")
        await eventually("the queue to empty") { await self.agent("e2e-msg")?.view.pendingInterject == 0 }
        await eventually("the read receipt") {
            self.phone.events.contains {
                if case .interjectConsumed(_, let agentKey) = $0 { return agentKey == item.agentKey }
                return false
            }
        }
    }

    func testQueuedMessagesCanBeReadBackAndWithdrawn() async throws {
        try await pair()
        let item = try await startWorking("e2e-recall")
        _ = try await phone.engine.sendInterject(
            macId: item.macId, agentKey: item.agentKey, text: "First thought", files: [], clientMsgId: nil, progress: { _ in }
        )
        _ = try await phone.engine.sendInterject(
            macId: item.macId, agentKey: item.agentKey, text: "Second thought", files: [], clientMsgId: nil, progress: { _ in }
        )
        let info = try await phone.engine.queryInterject(macId: item.macId, agentKey: item.agentKey)
        XCTAssertEqual(info.pending, 2)
        let text = try unwrap(info.text)
        XCTAssertTrue(text.contains("First thought") && text.contains("Second thought"), text)

        let left = try await phone.engine.clearInterject(macId: item.macId, agentKey: item.agentKey)
        XCTAssertEqual(left, 0)
        await eventually("the queue to empty") { await self.agent("e2e-recall")?.view.pendingInterject == 0 }
        let step = try await hook("activity", session: "e2e-recall", extra: toolStep())
        XCTAssertFalse(step.stdout.contains("thought"), "a withdrawn message is never delivered: \(step.stdout)")
    }

    func testAMessageToAnAgentThatIsNotWorkingIsRefused() async throws {
        try await pair()
        try await hook("session-start", session: "e2e-quiet")
        await eventually("the session") { await self.agent("e2e-quiet") != nil }
        let item = try unwrap(await agent("e2e-quiet"))
        do {
            _ = try await phone.engine.sendInterject(
                macId: item.macId, agentKey: item.agentKey, text: "Hello?", files: [], clientMsgId: nil, progress: { _ in }
            )
            XCTFail("an idle Agent takes no messages")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "notWorking")
        }
    }

    func testAMessageWithAnAttachmentReachesTheAgentAsAFile() async throws {
        try await pair()
        let item = try await startWorking("e2e-file")
        let payload = Data((0..<50_000).map { UInt8($0 % 241) })
        let outcome = try await phone.engine.sendInterject(
            macId: item.macId, agentKey: item.agentKey, text: "See the file",
            files: [UploadSource(name: "data.bin", mime: "application/octet-stream", data: payload, isImage: false)], clientMsgId: nil,
            progress: { _ in }
        )
        XCTAssertEqual(outcome.status, "queued")
        let step = try await hook("activity", session: "e2e-file", extra: toolStep())
        XCTAssertTrue(step.stdout.contains("data.bin"), "hook output: \(step.stdout)")
    }

    // MARK: new tasks

    func testANewTaskIsRefusedWhileTheMacHasTasksSwitchedOff() async throws {
        try await pair()
        let macId = try await macId()
        await phone.engine.loadTaskOptions(macId: macId)
        await eventually("the task options") {
            if case .ready = await self.phone.state().taskOptions[macId] { return true }
            return false
        }
        guard case .ready(let options)? = await phone.state().taskOptions[macId] else { return XCTFail("no options") }
        XCTAssertFalse(options.enabled)
        XCTAssertEqual(options.blocked, "disabled")

        let draft = TaskDraft(workspaceId: "deadbeefdeadbeef", workspaceName: "Nowhere", agentKind: "claude", agentLabel: "Claude Code", permission: "default", text: "Do something")
        let id = try await phone.engine.createTask(macId: macId, draft: draft)
        await eventually("the task to fail") { await self.phone.state().tasks.first { $0.id == id }?.stage == "failed" }
        let task = try unwrap(await phone.state().tasks.first { $0.id == id })
        XCTAssertNotNil(task.reason, "the Mac says why")
        XCTAssertFalse(task.canRetry, "a refusal is not a lost connection")
        let started = await phone.state().agents
        XCTAssertTrue(started.isEmpty, "nothing was started")
        await phone.engine.dismissTask(id: id)
        await eventually("the task to go") { await self.phone.state().tasks.isEmpty }
    }

    // MARK: question slot

    func testThePhoneCanTakeTheQuestionSlot() async throws {
        try await pair()
        let macId = try await macId()
        try await phone.engine.takeSlot(macId: macId)
        await eventually("the slot on this phone") { await self.phone.state().macs.first?.slot?.active == true }
    }
}
