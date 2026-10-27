import Foundation
import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The Agent side of the fake Mac: the per-phone list with its deltas, Follow, live frames,
/// interjections and new tasks.
extension TestMac {
    // MARK: sending

    func sendAgentsSnapshot(to phoneId: String) async {
        let (views, rev) = lock.withLock { (agents.values.sorted { $0.key < $1.key }, agentsRev) }
        await sendDown(AgentsSnapshot(epoch: epoch, rev: rev, agents: views), to: phoneId)
    }

    func sendFollowState(to phoneId: String, error: String? = nil) async {
        let (keys, rev) = lock.withLock { (followKeys, followRev) }
        await sendDown(FollowState(rev: rev, keys: keys, error: error), to: phoneId)
    }

    /// Changes the Agent list and tells every phone with the next revision.
    func changeAgents(upsert: [AgentView] = [], remove: [String] = []) async {
        let rev: UInt64 = lock.withLock {
            for view in upsert { agents[view.key] = view }
            for key in remove { agents[key] = nil }
            agentsRev += 1
            return agentsRev
        }
        for phone in pairedPhones { await sendDown(AgentsDelta(rev: rev, upsert: upsert, remove: remove), to: phone) }
    }

    /// Bumps the Agent revision without telling the phone (a lost delta).
    func skipAgentsRevision() {
        lock.withLock { agentsRev += 1 }
    }

    func sendFollowEvent(_ event: FollowEvent) async {
        for phone in pairedPhones { await sendDown(event, to: phone) }
    }

    func sendWatchFrame(agentKey: String, seq: UInt64, text: String, phase: String = "working") async {
        let frame: JSONValue = .object([
            "type": .string("watchFrame"), "seq": .int(Int64(seq)), "phase": .string(phase), "title": .string("Fix it"),
            "project": .string("HumanInLoop"), "text": .string(text),
            "steps": .array([.object(["kind": .string("run"), "name": .null, "object": .string("pnpm test"), "state": .string("running")])]),
            "stepsOmitted": .int(2),
            "todos": .array([.object(["content": .string("Fix the test"), "state": .string("inProgress")])]),
            "activeElapsedSecs": .int(61), "at": .int(1_800_000_000),
        ])
        for phone in pairedPhones { await sendDown(WatchFrame(agentKey: agentKey, seq: seq, frame: frame), to: phone) }
    }

    func sendWatchClosed(agentKey: String, reason: String) async {
        for phone in pairedPhones { await sendDown(WatchClosed(agentKey: agentKey, reason: reason), to: phone) }
    }

    func sendInterjectConsumed(agentKey: String) async {
        for phone in pairedPhones { await sendDown(InterjectConsumed(agentKey: agentKey), to: phone) }
    }

    func sendTaskStatus(_ id: String, stage: String, reason: String? = nil, agentKey: String? = nil) async {
        for phone in pairedPhones {
            await sendDown(TaskStatus(clientMsgId: id, stage: stage, reason: reason, agentKey: agentKey), to: phone)
        }
    }

    // MARK: handling

    func handleAgents(_ plain: Plain, from phoneId: String) async {
        switch plain.t {
        case FollowSet.type:
            guard let set = try? plain.body(as: FollowSet.self) else { return }
            let refused: Bool = lock.withLock {
                if set.on, !followKeys.contains(set.agentKey) {
                    if followKeys.count >= Wire.maxFollow { return true }
                    followKeys.append(set.agentKey)
                    followRev += 1
                } else if !set.on, followKeys.contains(set.agentKey) {
                    followKeys.removeAll { $0 == set.agentKey }
                    followRev += 1
                }
                return false
            }
            await sendFollowState(to: phoneId, error: refused ? "limit" : nil)
        case WatchOpen.type:
            guard let open = try? plain.body(as: WatchOpen.self) else { return }
            lock.withLock { watchOpens.append(open.agentKey) }
            await sendWatchFrame(agentKey: open.agentKey, seq: 1, text: "First frame")
        case WatchClose.type:
            guard let close = try? plain.body(as: WatchClose.self) else { return }
            lock.withLock { watchCloses.append(close.agentKey) }
        case InterjectSend.type:
            guard let send = try? plain.body(as: InterjectSend.self), let reply = interjectReply else { return }
            if reply.status != "rejected" { lock.withLock { interjectQueue.append(send.text) } }
            await sendDown(
                InterjectAck(clientMsgId: send.clientMsgId, agentKey: send.agentKey, status: reply.status, reason: reply.reason, pending: reply.pending),
                to: phoneId
            )
        case InterjectClear.type:
            guard let clear = try? plain.body(as: InterjectClear.self) else { return }
            lock.withLock { interjectQueue = [] }
            await sendDown(InterjectState(agentKey: clear.agentKey, pending: 0, text: nil), to: phoneId)
        case InterjectQuery.type:
            guard let query = try? plain.body(as: InterjectQuery.self) else { return }
            let queued = lock.withLock { interjectQueue }
            await sendDown(
                InterjectState(agentKey: query.agentKey, pending: UInt64(queued.count), text: queued.joined(separator: "\n\n")), to: phoneId
            )
        case TaskOptionsGet.type:
            guard !ignoreTaskOptions else { return }
            await sendDown(taskOptions, to: phoneId)
        case TaskTodosGet.type:
            guard let get = try? plain.body(as: TaskTodosGet.self) else { return }
            await sendDown(TaskTodos(workspaceId: get.workspaceId, todos: todos, overflow: 0), to: phoneId)
        case TaskCreate.type:
            guard !ignoreTasks, let create = try? plain.body(as: TaskCreate.self) else { return }
            for step in taskScript {
                await sendDown(TaskStatus(clientMsgId: create.clientMsgId, stage: step.stage, reason: step.reason, agentKey: step.agentKey), to: phoneId)
            }
        case SlotTake.type:
            lock.withLock {
                slotActive = true
                slotRev += 1
            }
            await sendDown(SlotState(rev: slotRev, active: true, autoActivation: false), to: phoneId)
        default:
            break
        }
    }
}

// MARK: - samples

func sampleAgent(
    _ key: String = "claude:s1", title: String = "Fix flaky tests", state: String = "working", followed: Bool = false,
    interject: Bool = true, pending: UInt64 = 0, lastActivityAt: Int64 = 1_800_000_000_000
) -> AgentView {
    let parts = key.split(separator: ":", maxSplits: 1).map(String.init)
    return AgentView(
        key: key, kind: parts[0], kindLabel: parts[0].capitalized, sessionId: parts[1], title: title,
        project: ProjectRef(key: "proj", name: "HumanInLoop"), state: state, startedAt: 1_799_999_000_000,
        lastActivityAt: lastActivityAt, activeElapsedSecs: 60, turnSteps: 3, currentTool: state == "working" ? "Edit" : nil,
        pendingInterject: pending, canInterject: CanInterject(ok: interject, reason: interject ? nil : "notWorking"), followed: followed
    )
}
