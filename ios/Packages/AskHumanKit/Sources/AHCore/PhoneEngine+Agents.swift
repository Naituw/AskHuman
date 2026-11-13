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

/// What the engine knows of how an open live view is doing, to notice one that has gone quiet while its Agent has not.
struct WatchHealth: Sendable {
    /// When the view was asked for (again): its first frame is due soon after.
    var openedAt: Date
    var lastFrameAt: Date?
    /// The last time the Agent list changed in a way a frame shows (state, title, steps), until a frame comes.
    var changedAt: Date?
    /// When the view was last asked for again by the watchdog.
    var askedAt: Date?
    /// The watchdog asked, and the Mac's answer has not come: what the view showed meanwhile, to tell whether it was news.
    var asking = false
    var shownWhenAsked: WatchFrameData?

    init(openedAt: Date) { self.openedAt = openedAt }
}

extension PhoneEngine {
    /// How often the open live views are looked at.
    static let watchdogInterval: TimeInterval = 5
    /// A view that was asked for and has had no frame after this long is asked for again.
    static let firstFrameWait: TimeInterval = 5
    /// A view whose Agent changed this long ago without a frame since is asked for again.
    static let silentFrameWait: TimeInterval = 20
    /// The watchdog asks for the same view again at most this often.
    static let watchdogSpacing: TimeInterval = 60

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
        for view in delta.upsert {
            let id = AgentItem.key(macId, view.key)
            if wantedWatches.contains(id), Self.changesTheFrame(from: mac.agents[view.key], to: view) {
                watchHealth[id]?.changedAt = clock.now()
            }
            mac.agents[view.key] = view
        }
        macs[macId] = mac
    }

    /// Whether the change of an Agent in the list is one a live view shows too, so that a frame is due after it.
    static func changesTheFrame(from old: AgentView?, to new: AgentView) -> Bool {
        guard let old else { return false }
        return old.state != new.state || old.title != new.title || old.turnSteps != new.turnSteps || old.currentTool != new.currentTool
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
        guard foreground, wantedWatches.contains(id) else {
            // Frames nobody asks for any more (the screen is gone): stop them.
            await send(WatchClose(agentKey: message.agentKey), to: macId)
            return
        }
        if let current = watches[id], message.seq <= current.seq, current.closed == nil { return }
        let data = WatchFrameData(json: message.frame)
        watches[id] = WatchState(
            macId: macId, agentKey: message.agentKey, seq: message.seq, frame: data, closed: nil, receivedAt: clock.now()
        )
        noteFrame(of: id, data)
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
        guard foreground else { return }
        for id in wantedWatches.sorted() where id.hasPrefix(macId + "|") {
            // A Mac that restarted numbers its frames from 1 again: they are not old frames to be dropped, and without this
            // the page would stay as it was until the user left it and came back.
            watches[id]?.seq = 0
            watchHealth[id] = WatchHealth(openedAt: clock.now())
            await send(WatchOpen(agentKey: String(id.dropFirst(macId.count + 1))), to: macId)
        }
        armWatchdog()
    }

    func suspendWatches() async {
        for macId in macs.keys where macs[macId]?.sync == .synced {
            for id in wantedWatches where id.hasPrefix(macId + "|") {
                await send(WatchClose(agentKey: String(id.dropFirst(macId.count + 1))), to: macId)
            }
        }
        watches = [:]
        watchHealth = [:]
        cancelTimer("watchdog")
    }

    public func watch(macId: String, agentKey: String, open: Bool) async {
        let id = AgentItem.key(macId, agentKey)
        if open {
            wantedWatches.insert(id)
            watchHealth[id] = WatchHealth(openedAt: clock.now())
            if macs[macId]?.sync == .synced { await send(WatchOpen(agentKey: agentKey), to: macId) }
            armWatchdog()
        } else {
            wantedWatches.remove(id)
            watches[id] = nil
            watchHealth[id] = nil
            if macs[macId]?.sync == .synced { await send(WatchClose(agentKey: agentKey), to: macId) }
        }
        publish()
    }

    // MARK: the watchdog of the live views

    /// A live view that shows nothing, or that shows an Agent that has stopped moving while the list says it works, is a
    /// stream that broke somewhere unseen (a frame lost on the way, a Mac that forgot the view). Leaving the page and coming
    /// back repairs it, because it asks the Mac for the view again; the watchdog does that by itself, once a minute at most
    /// for a view, and writes down that it did (`Diagnostics`, category `live`) and whether the answer was news.
    func armWatchdog() {
        guard foreground, !wantedWatches.isEmpty, timers["watchdog"] == nil else { return }
        after("watchdog", Self.watchdogInterval) { engine in await engine.checkWatches() }
    }

    func checkWatches() async {
        defer { armWatchdog() }
        guard foreground else { return }
        let now = clock.now()
        for id in wantedWatches.sorted() {
            guard let health = watchHealth[id], watches[id]?.closed == nil else { continue }
            if let askedAt = health.askedAt, now.timeIntervalSince(askedAt) < Self.watchdogSpacing { continue }
            let reason: String
            if health.lastFrameAt != nil {
                // `changedAt` only stays set while no frame has come since the change.
                guard let changedAt = health.changedAt, now.timeIntervalSince(changedAt) >= Self.silentFrameWait else { continue }
                reason = "the Agent changed \(Int(now.timeIntervalSince(changedAt))) s ago and no frame came"
            } else {
                guard now.timeIntervalSince(health.openedAt) >= Self.firstFrameWait else { continue }
                reason = "no first frame \(Int(now.timeIntervalSince(health.openedAt))) s after the view was asked for"
            }
            await askForTheViewAgain(id, because: reason, spacing: Self.watchdogSpacing)
        }
    }

    /// Sends `watch.open` again for a wanted view: the Mac answers with a whole frame, words included.
    func askForTheViewAgain(_ id: String, because reason: String, spacing: TimeInterval) async {
        guard foreground, wantedWatches.contains(id), var health = watchHealth[id],
              let split = id.firstIndex(of: "|") else { return }
        let macId = String(id[..<split])
        guard macs[macId]?.sync == .synced else { return }
        let now = clock.now()
        if let askedAt = health.askedAt, now.timeIntervalSince(askedAt) < spacing { return }
        health.askedAt = now
        health.openedAt = now
        health.asking = true
        health.shownWhenAsked = watches[id]?.frame
        watchHealth[id] = health
        Diagnostics.note("live", "a live view is asked for again: \(reason)")
        await send(WatchOpen(agentKey: String(id[id.index(after: split)...])), to: macId)
    }

    /// A frame came for a view: it is alive, and a question the watchdog asked is answered.
    func noteFrame(of id: String, _ data: WatchFrameData?) {
        guard var health = watchHealth[id] else { return }
        health.lastFrameAt = clock.now()
        health.changedAt = nil
        if health.asking {
            health.asking = false
            let verdict = Self.isNews(data, after: health.shownWhenAsked) ? "news" : "nothing new"
            Diagnostics.note("live", "the answer to the question had \(verdict)")
            health.shownWhenAsked = nil
        }
        watchHealth[id] = health
    }

    /// Whether a frame says anything the view did not show: the clocks in it do not count.
    static func isNews(_ frame: WatchFrameData?, after shown: WatchFrameData?) -> Bool {
        guard var fresh = frame, var old = shown else { return frame != nil || shown != nil }
        fresh.at = nil
        fresh.activeElapsedSecs = nil
        old.at = nil
        old.activeElapsedSecs = nil
        return fresh != old
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
        let message = InterjectSend(clientMsgId: id, agentKey: agentKey, text: text, files: uploadIds)
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
