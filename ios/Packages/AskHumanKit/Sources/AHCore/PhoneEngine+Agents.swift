import Foundation
import AHProtocol
import AHStore

// Agents (plan §4.4 – §4.6): the per-phone list with its deltas, Follow, live Watch frames and
// interjections. Like the inbox, the list is a stream of one snapshot and ordered deltas for the
// epoch the handshake named; a gap asks the Mac for a fresh snapshot.

/// Replies of one conversation with a Mac, queued until the sender asks for them.
struct ReplyMailbox<Reply: Sendable> {
    var owner: String
    var replies: [Reply] = []
    var waiter: CheckedContinuation<Reply, Error>?

    init(owner: String) { self.owner = owner }

    /// Hands the reply to the task that waits, or keeps it for the one that will.
    mutating func deliver(_ reply: Reply) {
        if let waiter {
            self.waiter = nil
            waiter.resume(returning: reply)
        } else {
            replies.append(reply)
        }
    }

    mutating func popReply() -> Reply? {
        replies.isEmpty ? nil : replies.removeFirst()
    }

    /// Wakes the waiting task with an error; does nothing when nobody waits.
    mutating func fail(_ error: Error) {
        guard let waiter else { return }
        self.waiter = nil
        waiter.resume(throwing: error)
    }
}

extension PhoneEngine {
    /// How long an interjection waits for the Mac's acknowledgement.
    static let interjectAckTimeout: TimeInterval = 20
    /// How long a query or clear waits for the Mac's `interject.state`.
    static let interjectStateTimeout: TimeInterval = 8

    // MARK: list

    func onAgentsSnapshot(_ macId: String, _ snapshot: AgentsSnapshot) async {
        guard var mac = macs[macId], snapshot.epoch == mac.epoch else { return }
        mac.agents = Dictionary(snapshot.agents.map { ($0.key, $0) }, uniquingKeysWith: { _, last in last })
        mac.agentsRev = snapshot.rev
        macs[macId] = mac
        await reopenWatches(macId)
    }

    func onAgentsDelta(_ macId: String, _ delta: AgentsDelta) async {
        guard var mac = macs[macId], mac.epoch != nil else { return }
        guard let last = mac.agentsRev else {
            await requestResync(macId, stream: "agents")
            return
        }
        if delta.rev <= last { return }
        if delta.rev != last + 1 {
            await requestResync(macId, stream: "agents")
            return
        }
        mac.agentsRev = delta.rev
        for key in delta.remove { mac.agents[key] = nil }
        for view in delta.upsert { mac.agents[view.key] = view }
        macs[macId] = mac
    }

    /// Orders the list: questions first, then running, idle and finished sessions; the most
    /// recent activity first within each.
    static func agentOrder(_ a: AgentItem, _ b: AgentItem) -> Bool {
        func rank(_ state: String) -> Int {
            switch state {
            case "waiting": return 0
            case "working": return 1
            case "idle": return 2
            default: return 3
            }
        }
        let (ra, rb) = (rank(a.view.state), rank(b.view.state))
        if ra != rb { return ra < rb }
        if a.view.lastActivityAt != b.view.lastActivityAt { return a.view.lastActivityAt > b.view.lastActivityAt }
        return a.id < b.id
    }

    func agentItems() -> [AgentItem] {
        var items: [AgentItem] = []
        for mac in macs.values where mac.sync == .synced {
            for var view in mac.agents.values {
                // The Mac's own Follow list is the truth once it was sent (the flag in the list can lag).
                if mac.followRev != nil { view.followed = mac.follow.contains(view.key) }
                items.append(AgentItem(macId: mac.record.macId, macName: mac.record.displayName, view: view))
            }
        }
        return items.sorted(by: Self.agentOrder)
    }

    // MARK: follow

    func onFollowState(_ macId: String, _ state: FollowState) {
        guard var mac = macs[macId], mac.epoch != nil else { return }
        if let last = mac.followRev, state.rev < last { return }
        mac.followRev = state.rev
        mac.follow = Set(state.keys)
        macs[macId] = mac
        var meta = store.meta(macId: macId)
        meta.follow = state.keys
        store.saveMeta(meta, macId: macId)
        if state.error != nil, let key = followRequests.removeValue(forKey: macId) {
            emit(.followRefused(macId: macId, agentKey: key))
        } else {
            followRequests[macId] = nil
        }
    }

    func onFollowEvent(_ macId: String, _ event: FollowEvent) {
        emit(.followEvent(macId: macId, event: event))
    }

    public func setFollow(macId: String, agentKey: String, on: Bool) async throws {
        guard var mac = macs[macId], mac.sync == .synced else { throw BackendError.macOffline }
        if on, !mac.follow.contains(agentKey), mac.follow.count >= Wire.maxFollow { throw BackendError.rejected("limit") }
        // Show the change at once; the Mac's `follow.state` has the last word.
        if on { mac.follow.insert(agentKey) } else { mac.follow.remove(agentKey) }
        if mac.followRev == nil { mac.followRev = 0 }
        macs[macId] = mac
        followRequests[macId] = on ? agentKey : nil
        publish()
        let sent = await send(FollowSet(clientMsgId: Bytes.newMsgId(), agentKey: agentKey, on: on), to: macId)
        if !sent { throw BackendError.macOffline }
    }

    // MARK: watch

    func onWatchFrame(_ macId: String, _ message: WatchFrame) async {
        let id = AgentItem.key(macId, message.agentKey)
        guard wantedWatches.contains(id) else {
            // Frames nobody asks for any more (the screen is gone): stop them.
            await send(WatchClose(agentKey: message.agentKey), to: macId)
            return
        }
        if let current = watches[id], message.seq <= current.seq, current.closed == nil { return }
        watches[id] = WatchState(
            macId: macId, agentKey: message.agentKey, seq: message.seq, frame: WatchFrameData(json: message.frame),
            closed: nil, receivedAt: clock.now()
        )
    }

    func onWatchClosed(_ macId: String, _ message: WatchClosed) {
        let id = AgentItem.key(macId, message.agentKey)
        guard wantedWatches.contains(id) else { return }
        var state = watches[id] ?? WatchState(macId: macId, agentKey: message.agentKey, receivedAt: clock.now())
        state.closed = message.reason
        state.receivedAt = clock.now()
        watches[id] = state
    }

    /// Asks the Mac for the frames of every watch the user still has open (after a reconnect the
    /// Mac knows none of them).
    func reopenWatches(_ macId: String) async {
        for id in wantedWatches.sorted() where id.hasPrefix(macId + "|") {
            await send(WatchOpen(agentKey: String(id.dropFirst(macId.count + 1))), to: macId)
        }
    }

    public func watch(macId: String, agentKey: String, open: Bool) async {
        let id = AgentItem.key(macId, agentKey)
        if open {
            wantedWatches.insert(id)
            if macs[macId]?.sync == .synced { await send(WatchOpen(agentKey: agentKey), to: macId) }
        } else {
            wantedWatches.remove(id)
            watches[id] = nil
            if macs[macId]?.sync == .synced { await send(WatchClose(agentKey: agentKey), to: macId) }
        }
        publish()
    }

    // MARK: interjections

    func onInterjectAck(_ macId: String, _ ack: InterjectAck) {
        guard interjectBoxes[ack.clientMsgId]?.owner == macId else { return }
        interjectBoxes[ack.clientMsgId]?.deliver(ack)
    }

    func onInterjectState(_ macId: String, _ state: InterjectState) {
        let key = AgentItem.key(macId, state.agentKey)
        guard interjectStateBoxes[key]?.owner == macId else { return }
        interjectStateBoxes[key]?.deliver(state)
    }

    func onInterjectConsumed(_ macId: String, _ message: InterjectConsumed) {
        emit(.interjectConsumed(macId: macId, agentKey: message.agentKey))
    }

    public func sendInterject(
        macId: String, agentKey: String, text: String, files: [UploadSource], clientMsgId: String?,
        progress: @escaping @Sendable (Double) -> Void
    ) async throws -> InterjectOutcome {
        try await sendAgentMessage(macId: macId, agentKey: agentKey, text: text, files: files, clientMsgId: clientMsgId, expectedResultId: nil, progress: progress)
    }

    public func continueAgent(macId: String, agentKey: String, expectedResultId: String, clientMsgId: String) async throws -> InterjectOutcome {
        try await sendAgentMessage(macId: macId, agentKey: agentKey, text: "Continue", files: [], clientMsgId: clientMsgId, expectedResultId: expectedResultId, progress: { _ in })
    }

    private func sendAgentMessage(
        macId: String, agentKey: String, text: String, files: [UploadSource], clientMsgId: String?, expectedResultId: String?,
        progress: @escaping @Sendable (Double) -> Void
    ) async throws -> InterjectOutcome {
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }
        let id = clientMsgId ?? Bytes.newMsgId()
        var uploadIds: [String] = []
        for (index, file) in files.enumerated() {
            let done = Double(index)
            let total = Double(files.count)
            uploadIds.append(try await performUpload(
                macId: macId, scope: UploadScope(agentKey: agentKey), source: file,
                progress: { fraction in progress((done + fraction) / total) }
            ))
        }
        let message = InterjectSend(clientMsgId: id, agentKey: agentKey, text: text, files: uploadIds, expectedResultId: expectedResultId)
        guard interjectBoxes[id] == nil else { throw BackendError.unavailable("busy") }
        interjectBoxes[id] = ReplyMailbox(owner: macId)
        defer {
            interjectBoxes[id] = nil
            cancelTimer("iack." + id)
        }
        guard await send(message, to: macId) else { throw BackendError.macOffline }
        let ack = try await nextInterjectAck(id)
        if ack.status == "rejected" { throw BackendError.rejected(ack.reason ?? "rejected") }
        return InterjectOutcome(status: ack.status, pending: Int(ack.pending), notificationRegistered: ack.notificationRegistered)
    }

    func nextInterjectAck(_ id: String) async throws -> InterjectAck {
        if let queued = interjectBoxes[id]?.popReply() { return queued }
        after("iack." + id, Self.interjectAckTimeout) { engine in await engine.failInterjectWait(id, with: BackendError.timedOut) }
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<InterjectAck, Error>) in
                if interjectBoxes[id] != nil {
                    interjectBoxes[id]?.waiter = cont
                } else {
                    cont.resume(throwing: BackendError.cancelled)
                }
            }
        } onCancel: {
            Task { await self.failInterjectWait(id, with: BackendError.cancelled) }
        }
    }

    func failInterjectWait(_ id: String, with error: BackendError) {
        interjectBoxes[id]?.fail(error)
    }

    public func clearInterject(macId: String, agentKey: String) async throws -> Int {
        let state = try await interjectStateRequest(
            macId: macId, agentKey: agentKey, message: InterjectClear(clientMsgId: Bytes.newMsgId(), agentKey: agentKey)
        )
        return Int(state.pending)
    }

    public func queryInterject(macId: String, agentKey: String) async throws -> InterjectInfo {
        let state = try await interjectStateRequest(macId: macId, agentKey: agentKey, message: InterjectQuery(agentKey: agentKey))
        return InterjectInfo(pending: Int(state.pending), text: state.text)
    }

    func interjectStateRequest<M: WireMessage>(macId: String, agentKey: String, message: M) async throws -> InterjectState {
        guard macs[macId]?.sync == .synced else { throw BackendError.macOffline }
        let key = AgentItem.key(macId, agentKey)
        guard interjectStateBoxes[key] == nil else { throw BackendError.unavailable("busy") }
        interjectStateBoxes[key] = ReplyMailbox(owner: macId)
        defer {
            interjectStateBoxes[key] = nil
            cancelTimer("istate." + key)
        }
        guard await send(message, to: macId) else { throw BackendError.macOffline }
        return try await nextInterjectState(key)
    }

    func nextInterjectState(_ key: String) async throws -> InterjectState {
        if let queued = interjectStateBoxes[key]?.popReply() { return queued }
        after("istate." + key, Self.interjectStateTimeout) { engine in await engine.failInterjectStateWait(key, with: BackendError.timedOut) }
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<InterjectState, Error>) in
                if interjectStateBoxes[key] != nil {
                    interjectStateBoxes[key]?.waiter = cont
                } else {
                    cont.resume(throwing: BackendError.cancelled)
                }
            }
        } onCancel: {
            Task { await self.failInterjectStateWait(key, with: BackendError.cancelled) }
        }
    }

    func failInterjectStateWait(_ key: String, with error: BackendError) {
        interjectStateBoxes[key]?.fail(error)
    }

    /// Fails the waiters of the interjections of a Mac that went away (all Macs when `macId` is `nil`).
    func failInterjects(for macId: String?, with error: BackendError) {
        for (id, box) in interjectBoxes where macId == nil || box.owner == macId {
            interjectBoxes[id]?.fail(error)
        }
        for (key, box) in interjectStateBoxes where macId == nil || box.owner == macId {
            interjectStateBoxes[key]?.fail(error)
        }
    }
}
