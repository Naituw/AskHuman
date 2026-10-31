import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

final class AgentTests: XCTestCase {
    private func agentId(_ h: Harness, _ key: String) -> String { AgentItem.key(h.mac.macId, key) }

    private func resyncs(_ h: Harness, _ stream: String) -> Int {
        h.mac.received(Wire.T.resync).filter { (try? $0.body(as: Resync.self).stream) == stream }.count
    }

    // MARK: list

    func testTheAgentListArrivesWithTheHandshakeAndFollowsDeltas() async throws {
        let h = Harness()
        h.mac.agents = [
            "claude:s1": sampleAgent("claude:s1", state: "working", lastActivityAt: 1_800_000_000_000),
            "cursor:s2": sampleAgent("cursor:s2", title: "Needs an answer", state: "waiting", lastActivityAt: 1_700_000_000_000),
            "codex:s3": sampleAgent("codex:s3", state: "idle", interject: false),
        ]
        h.mac.followKeys = ["codex:s3"]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 3 }

        var agents = await h.state().agents
        XCTAssertEqual(agents.map(\.agentKey), ["cursor:s2", "claude:s1", "codex:s3"], "waiting first, then working, then idle")
        XCTAssertEqual(agents.first { $0.agentKey == "codex:s3" }?.view.followed, true, "the Mac's follow list decides")
        XCTAssertEqual(agents.first { $0.agentKey == "claude:s1" }?.view.followed, false)
        XCTAssertEqual(agents[0].macName, "Home Mac")

        await h.mac.changeAgents(upsert: [sampleAgent("claude:s1", title: "Renamed", state: "idle", interject: false), sampleAgent("pi:s4")], remove: ["cursor:s2"])
        await eventually("the delta") { await h.state().agents.contains { $0.agentKey == "pi:s4" } }
        agents = await h.state().agents
        XCTAssertEqual(Set(agents.map(\.agentKey)), ["claude:s1", "codex:s3", "pi:s4"])
        XCTAssertEqual(agents.first { $0.agentKey == "claude:s1" }?.view.title, "Renamed")
        XCTAssertEqual(resyncs(h, "agents"), 0)
    }

    func testAMissedDeltaAsksForAFreshSnapshot() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }

        h.mac.skipAgentsRevision()
        await h.mac.changeAgents(upsert: [sampleAgent("pi:s4")])
        await eventually("an agents resync") { self.resyncs(h, "agents") == 1 }
        await eventually("the snapshot repairs the list") { await h.state().agents.count == 2 }
    }

    func testAnOldDeltaIsDroppedWithoutAResync() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }
        let phone = h.mac.pairedPhones[0]
        await h.mac.sendDown(AgentsDelta(rev: 1, upsert: [sampleAgent("pi:old")], remove: []), to: phone)
        try await Task.sleep(nanoseconds: 60_000_000)
        let keys = await h.state().agents.map(\.agentKey)
        XCTAssertEqual(keys, ["claude:s1"])
        XCTAssertEqual(resyncs(h, "agents"), 0)
    }

    func testAnOfflineMacHasNoAgentsAndTheyComeBack() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }

        await h.mac.publishPresence(online: false)
        await eventually("no agents while offline") { await h.state().agents.isEmpty }
        await h.mac.publishPresence(online: true)
        await eventually("agents are back") { await h.state().agents.count == 1 }
    }

    // MARK: follow

    func testFollowingAnAgentTellsTheMacAndShowsItsAnswer() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }

        try await h.engine.setFollow(macId: h.mac.macId, agentKey: "claude:s1", on: true)
        await eventually("follow.set") { h.mac.count(FollowSet.type) == 1 }
        let sent = try h.mac.received(FollowSet.type)[0].body(as: FollowSet.self)
        XCTAssertEqual(sent.agentKey, "claude:s1")
        XCTAssertTrue(sent.on)
        await eventually("followed") { await h.state().agents.first?.view.followed == true }
        XCTAssertEqual(h.store.meta(macId: h.mac.macId).follow, ["claude:s1"], "remembered on the phone")

        try await h.engine.setFollow(macId: h.mac.macId, agentKey: "claude:s1", on: false)
        await eventually("unfollowed") { await h.state().agents.first?.view.followed == false }
    }

    func testTheFollowLimitIsCheckedOnThePhoneFirst() async throws {
        let h = Harness()
        var agents: [String: AgentView] = [:]
        for index in 0..<6 { agents["claude:s\(index)"] = sampleAgent("claude:s\(index)", followed: index < 5) }
        h.mac.agents = agents
        h.mac.followKeys = (0..<5).map { "claude:s\($0)" }
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 6 }

        do {
            try await h.engine.setFollow(macId: h.mac.macId, agentKey: "claude:s5", on: true)
            XCTFail("the sixth follow must be refused")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "limit")
        }
        XCTAssertEqual(h.mac.count(FollowSet.type), 0, "nothing was sent")
    }

    func testARefusalOfTheMacRollsTheFollowBackAndIsReported() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }
        // The Mac follows five others (set on another phone): the phone does not know yet.
        h.mac.followKeys = (10..<15).map { "claude:s\($0)" }

        try await h.engine.setFollow(macId: h.mac.macId, agentKey: "claude:s1", on: true)
        await eventually("the refusal") {
            h.events.contains(.followRefused(macId: h.mac.macId, agentKey: "claude:s1"))
        }
        await eventually("not followed") { await h.state().agents.first?.view.followed == false }
    }

    func testFollowEventsReachTheUI() async throws {
        let h = Harness()
        await h.pair()
        let event = FollowEvent(agentKey: "claude:s1", kind: "done", title: "Fix flaky tests", project: "HumanInLoop", preview: "All green")
        await h.mac.sendFollowEvent(event)
        await eventually("the event") { h.events.contains(.followEvent(macId: h.mac.macId, event: event)) }
    }

    // MARK: watch

    func testAWatchShowsFramesAndStopsWhenClosed() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }
        let id = agentId(h, "claude:s1")

        await h.engine.watch(macId: h.mac.macId, agentKey: "claude:s1", open: true)
        await eventually("the first frame") { await h.state().watches[id]?.frame?.text == "First frame" }
        var frame = try unwrap(await h.state().watches[id]?.frame)
        XCTAssertEqual(frame.phase, "working")
        XCTAssertEqual(frame.steps.first?.object, "pnpm test")
        XCTAssertEqual(frame.steps.first?.state, "running")
        XCTAssertEqual(frame.stepsOmitted, 2)
        XCTAssertEqual(frame.todos.first?.state, "inProgress")
        XCTAssertEqual(frame.activeElapsedSecs, 61)

        await h.mac.sendWatchFrame(agentKey: "claude:s1", seq: 2, text: "Second frame")
        await eventually("the second frame") { await h.state().watches[id]?.frame?.text == "Second frame" }
        // An older frame (reordered) does not overwrite a newer one.
        await h.mac.sendWatchFrame(agentKey: "claude:s1", seq: 1, text: "Stale frame")
        try await Task.sleep(nanoseconds: 60_000_000)
        frame = try unwrap(await h.state().watches[id]?.frame)
        XCTAssertEqual(frame.text, "Second frame")

        await h.engine.watch(macId: h.mac.macId, agentKey: "claude:s1", open: false)
        await eventually("watch.close") { h.mac.watchCloses == ["claude:s1"] }
        let watching = await h.state().watches[id]
        XCTAssertNil(watching)

        // A frame already on its way is answered with a close, not shown.
        await h.mac.sendWatchFrame(agentKey: "claude:s1", seq: 9, text: "Late frame")
        await eventually("a second watch.close") { h.mac.watchCloses.count == 2 }
        let late = await h.state().watches[id]
        XCTAssertNil(late)
    }

    func testAWatchTheUserKeepsIsOpenedAgainAfterTheMacRestarts() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }
        await h.engine.watch(macId: h.mac.macId, agentKey: "claude:s1", open: true)
        await eventually("watch.open") { h.mac.watchOpens.count == 1 }

        h.mac.epoch = "restarted"
        await h.mac.publishPresence(online: true)
        await eventually("watch.open again after the new handshake") { h.mac.watchOpens.count == 2 }
        await eventually("frames again") { await h.state().watches[self.agentId(h, "claude:s1")]?.frame != nil }
    }

    func testTheMacClosingAWatchIsShown() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }
        let id = agentId(h, "claude:s1")
        await h.engine.watch(macId: h.mac.macId, agentKey: "claude:s1", open: true)
        await eventually("the first frame") { await h.state().watches[id]?.frame != nil }

        await h.mac.sendWatchClosed(agentKey: "claude:s1", reason: "limit")
        await eventually("closed") { await h.state().watches[id]?.closed == "limit" }
        let kept = await h.state().watches[id]?.frame
        XCTAssertNotNil(kept, "the last frame stays on screen")
    }

    // MARK: interjections

    func testAnIdleDesktopChatAcceptsAMessageWithFilesAndNoPendingQueue() async throws {
        let h = Harness()
        h.mac.agents = ["codex:desktop": sampleAgent("codex:desktop", state: "idle", interject: true)]
        h.mac.interjectReply = ("delivered", nil, 0)
        await h.pair()
        await eventually("the idle Desktop chat") { await h.state().agents.first?.view.canInterject.ok == true }
        let source = UploadSource(name: "note.txt", mime: "text/plain", data: Data("note".utf8), isImage: false)
        let result = try await h.engine.sendInterject(
            macId: h.mac.macId, agentKey: "codex:desktop", text: "Continue", files: [source], clientMsgId: "desktop-message", progress: { _ in }
        )
        XCTAssertEqual(result, InterjectOutcome(status: "delivered", pending: 0))
        let sent = try h.mac.received(InterjectSend.type)[0].body(as: InterjectSend.self)
        XCTAssertEqual(sent.clientMsgId, "desktop-message")
        XCTAssertEqual(sent.agentKey, "codex:desktop")
        XCTAssertEqual(sent.text, "Continue")
        XCTAssertEqual(sent.files.count, 1)
    }

    func testAnInterjectionIsAcknowledgedAsQueued() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }

        let outcome = try await h.engine.sendInterject(
            macId: h.mac.macId, agentKey: "claude:s1", text: "Also run the linter", files: [], clientMsgId: nil, progress: { _ in }
        )
        XCTAssertEqual(outcome, InterjectOutcome(status: "queued", pending: 1))
        let sent = try h.mac.received(InterjectSend.type)[0].body(as: InterjectSend.self)
        XCTAssertEqual(sent.text, "Also run the linter")
        XCTAssertEqual(sent.agentKey, "claude:s1")
        XCTAssertTrue(sent.files.isEmpty)
    }

    func testARefusedInterjectionSaysWhy() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        h.mac.interjectReply = ("rejected", "notWorking", 0)
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }

        do {
            _ = try await h.engine.sendInterject(
                macId: h.mac.macId, agentKey: "claude:s1", text: "Hello", files: [], clientMsgId: nil, progress: { _ in }
            )
            XCTFail("the Mac refused")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "notWorking")
        }
    }

    func testAnInterjectionTheMacNeverAnswersTimesOutAndCanBeRepeatedWithTheSameId() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        h.mac.interjectReply = nil
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }

        let finished = DoneFlag()
        let outcome = Task {
            defer { finished.set() }
            return try await h.engine.sendInterject(
                macId: h.mac.macId, agentKey: "claude:s1", text: "Hello", files: [], clientMsgId: "fixed-id", progress: { _ in }
            )
        }
        await eventually("interject.send") { h.mac.count(InterjectSend.type) == 1 }
        await h.advance(until: { finished.value }, "the ack timeout")
        do {
            _ = try await outcome.value
            XCTFail("no answer, no outcome")
        } catch BackendError.timedOut {}

        h.mac.interjectReply = ("queued", nil, 1)
        let retried = try await h.engine.sendInterject(
            macId: h.mac.macId, agentKey: "claude:s1", text: "Hello", files: [], clientMsgId: "fixed-id", progress: { _ in }
        )
        XCTAssertEqual(retried.status, "queued")
        let ids = try h.mac.received(InterjectSend.type).map { try $0.body(as: InterjectSend.self).clientMsgId }
        XCTAssertEqual(ids, ["fixed-id", "fixed-id"], "the retry is idempotent on the Mac")
    }

    func testFilesForAnAgentTravelInTheAgentScope() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }

        let data = Data("log line\n".utf8)
        let source = UploadSource(name: "build.log", mime: "text/plain", data: data, isImage: false)
        let progress = ProgressLog()
        _ = try await h.engine.sendInterject(
            macId: h.mac.macId, agentKey: "claude:s1", text: "See the log", files: [source], clientMsgId: nil,
            progress: { progress.add($0) }
        )
        let begin = try h.mac.received(UploadBegin.type)[0].body(as: UploadBegin.self)
        XCTAssertEqual(begin.scope.agentKey, "claude:s1")
        XCTAssertNil(begin.scope.requestId)
        let sent = try h.mac.received(InterjectSend.type)[0].body(as: InterjectSend.self)
        XCTAssertEqual(sent.files, [begin.uploadId])
        XCTAssertEqual(h.mac.uploaded(begin.uploadId)?.data, data)
        XCTAssertEqual(progress.values.last, 1)
    }

    func testRecallingTakesTheWaitingTextBackAndClearsIt() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }
        _ = try await h.engine.sendInterject(macId: h.mac.macId, agentKey: "claude:s1", text: "First", files: [], clientMsgId: nil, progress: { _ in })

        let info = try await h.engine.queryInterject(macId: h.mac.macId, agentKey: "claude:s1")
        XCTAssertEqual(info, InterjectInfo(pending: 1, text: "First"))
        let left = try await h.engine.clearInterject(macId: h.mac.macId, agentKey: "claude:s1")
        XCTAssertEqual(left, 0)
        let after = try await h.engine.queryInterject(macId: h.mac.macId, agentKey: "claude:s1")
        XCTAssertEqual(after.pending, 0)
    }

    func testAnOfflineMacFailsInterjectionsRightAway() async throws {
        let h = Harness()
        h.mac.agents = ["claude:s1": sampleAgent("claude:s1")]
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 1 }
        await h.mac.publishPresence(online: false)
        await eventually("offline") { await h.state().macs.first?.link == .macOffline }
        do {
            _ = try await h.engine.sendInterject(macId: h.mac.macId, agentKey: "claude:s1", text: "x", files: [], clientMsgId: nil, progress: { _ in })
            XCTFail("offline")
        } catch BackendError.macOffline {}
    }

    func testTheMacReadingTheMessagesIsAnEvent() async throws {
        let h = Harness()
        await h.pair()
        await h.mac.sendInterjectConsumed(agentKey: "claude:s1")
        await eventually("consumed") { h.events.contains(.interjectConsumed(macId: h.mac.macId, agentKey: "claude:s1")) }
    }

    // MARK: tasks

    func testTheChoicesOfAMacAreLoadedOnRequest() async throws {
        let h = Harness()
        h.mac.todos = [TodoOption(id: "t1", text: "Write the docs", auto: false, attachments: 0)]
        await h.pair()

        await h.engine.loadTaskOptions(macId: h.mac.macId)
        await eventually("options") {
            if case .ready(let options)? = await h.state().taskOptions[h.mac.macId] { return options.workspaces.count == 1 }
            return false
        }
        await h.engine.loadTaskTodos(macId: h.mac.macId, workspaceId: "ws1")
        await eventually("todos") { await h.state().todos(macId: h.mac.macId, workspaceId: "ws1")?.first?.id == "t1" }
    }

    func testChoicesThatNeverComeAreReportedAsATimeout() async throws {
        let h = Harness()
        await h.pair()
        h.mac.ignoreTaskOptions = true
        await h.engine.loadTaskOptions(macId: h.mac.macId)
        await h.advance(until: {
            if case .failed? = await h.state().taskOptions[h.mac.macId] { return true }
            return false
        }, "the options timeout")
    }

    func testATaskGoesFromAcceptedToWatching() async throws {
        let h = Harness()
        h.mac.taskScript = [("accepted", nil, nil), ("launching", nil, nil), ("launched", nil, nil), ("watching", nil, "claude:new1")]
        await h.pair()
        let draft = TaskDraft(workspaceId: "ws1", workspaceName: "HumanInLoop", agentKind: "claude", agentLabel: "Claude Code", permission: "default", text: "Fix the build")

        let id = try await h.engine.createTask(macId: h.mac.macId, draft: draft)
        await eventually("watching") { await h.state().tasks.first?.stage == "watching" }
        let task = try unwrap(await h.state().tasks.first)
        XCTAssertEqual(task.id, id)
        XCTAssertEqual(task.agentKey, "claude:new1")
        XCTAssertTrue(task.isFinal)
        let create = try h.mac.received(TaskCreate.type)[0].body(as: TaskCreate.self)
        XCTAssertEqual(create.clientMsgId, id)
        XCTAssertEqual(create.workspaceId, "ws1")
        XCTAssertEqual(create.agentKind, "claude")
        XCTAssertEqual(create.permission, "default")
        XCTAssertEqual(create.text, "Fix the build")
        XCTAssertNil(create.todoId)
    }

    func testALateEarlierStageDoesNotUndoProgress() async throws {
        let h = Harness()
        h.mac.taskScript = [("accepted", nil, nil), ("launched", nil, nil)]
        await h.pair()
        let draft = TaskDraft(workspaceId: "ws1", workspaceName: "HumanInLoop", agentKind: "claude", agentLabel: "Claude Code", permission: "default", text: "Go")
        let id = try await h.engine.createTask(macId: h.mac.macId, draft: draft)
        await eventually("launched") { await h.state().tasks.first?.stage == "launched" }

        await h.mac.sendTaskStatus(id, stage: "accepted")
        try await Task.sleep(nanoseconds: 60_000_000)
        let stage = await h.state().tasks.first?.stage
        XCTAssertEqual(stage, "launched")
        await h.mac.sendTaskStatus(id, stage: "unmatched")
        await eventually("unmatched is final") { await h.state().tasks.first?.stage == "unmatched" }
    }

    func testAFailedTaskKeepsTheReason() async throws {
        let h = Harness()
        h.mac.taskScript = [("accepted", nil, nil), ("failed", "noTerminal", nil)]
        await h.pair()
        let draft = TaskDraft(workspaceId: "ws1", workspaceName: "HumanInLoop", agentKind: "claude", agentLabel: "Claude Code", permission: "default", text: "Go")
        _ = try await h.engine.createTask(macId: h.mac.macId, draft: draft)
        await eventually("failed") { await h.state().tasks.first?.stage == "failed" }
        let task = try unwrap(await h.state().tasks.first)
        XCTAssertEqual(task.reason, "noTerminal")
        XCTAssertFalse(task.canRetry, "a refusal is final: the form is sent again as a new task")
    }

    func testATaskTheMacNeverAcceptsCanBeSentAgainUnderTheSameId() async throws {
        let h = Harness()
        h.mac.ignoreTasks = true
        await h.pair()
        let draft = TaskDraft(workspaceId: "ws1", workspaceName: "HumanInLoop", agentKind: "codex", agentLabel: "Codex", permission: "yolo", speed: .fast, text: "Go")
        let id = try await h.engine.createTask(macId: h.mac.macId, draft: draft)
        await eventually("task.create") { h.mac.count(TaskCreate.type) == 1 }
        await h.advance(until: { await h.state().tasks.first?.stage == "failed" }, "the accept timeout")
        var task = try unwrap(await h.state().tasks.first)
        XCTAssertEqual(task.reason, "connection")
        XCTAssertTrue(task.canRetry)

        h.mac.ignoreTasks = false
        try await h.engine.retryTask(id: id)
        await eventually("launched") { await h.state().tasks.first?.stage == "launched" }
        let ids = try h.mac.received(TaskCreate.type).map { try $0.body(as: TaskCreate.self).clientMsgId }
        XCTAssertEqual(ids, [id, id])
        let speeds = try h.mac.received(TaskCreate.type).map { try $0.body(as: TaskCreate.self).speed }
        XCTAssertEqual(speeds, [.fast, .fast])
        XCTAssertEqual(task.speed, .fast)
        task = try unwrap(await h.state().tasks.first)
        XCTAssertNil(task.reason)
    }

    func testATaskInProgressIsMarkedLostWhenTheMacGoesAway() async throws {
        let h = Harness()
        h.mac.taskScript = [("accepted", nil, nil)]
        await h.pair()
        let draft = TaskDraft(workspaceId: "ws1", workspaceName: "HumanInLoop", agentKind: "claude", agentLabel: "Claude Code", permission: "default", text: "Go")
        _ = try await h.engine.createTask(macId: h.mac.macId, draft: draft)
        await eventually("accepted") { await h.state().tasks.first?.stage == "accepted" }

        await h.mac.publishPresence(online: false)
        await eventually("lost") { await h.state().tasks.first?.stage == "lost" }
        let task = try unwrap(await h.state().tasks.first)
        XCTAssertTrue(task.canRetry)
        XCTAssertFalse(task.isActive)
    }

    func testDismissingATaskForgetsIt() async throws {
        let h = Harness()
        await h.pair()
        let draft = TaskDraft(workspaceId: "ws1", workspaceName: "HumanInLoop", agentKind: "claude", agentLabel: "Claude Code", permission: "default", text: "Go")
        let id = try await h.engine.createTask(macId: h.mac.macId, draft: draft)
        await eventually("launched") { await h.state().tasks.first?.stage == "launched" }
        await h.engine.dismissTask(id: id)
        let tasks = await h.state().tasks
        XCTAssertTrue(tasks.isEmpty)
    }

    // MARK: slot

    func testTakingOverQuestionsAsksTheMacAndShowsTheNewState() async throws {
        let h = Harness()
        h.mac.slotActive = false
        await h.pair()
        await eventually("the slot state") { await h.state().macs.first?.slot?.active == false }

        try await h.engine.takeSlot(macId: h.mac.macId)
        await eventually("slot.take") { h.mac.count(SlotTake.type) == 1 }
        await eventually("active") { await h.state().macs.first?.slot?.active == true }
    }
}

final class DoneFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var flag = false
    var value: Bool { lock.withLock { flag } }
    func set() { lock.withLock { flag = true } }
}

final class ProgressLog: @unchecked Sendable {
    private let lock = NSLock()
    private var log: [Double] = []
    var values: [Double] { lock.withLock { log } }
    func add(_ value: Double) { lock.withLock { log.append(value) } }
}
