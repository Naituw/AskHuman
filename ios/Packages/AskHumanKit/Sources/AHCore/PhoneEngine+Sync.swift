import Foundation
import AHProtocol
import AHStore

// Handshake and stream synchronisation (plan §3.5): `hello` once the Mac is online, then the
// Mac's snapshots; deltas are applied only in order and only for the epoch the handshake named.

extension PhoneEngine {
    static let helloTimeout: TimeInterval = 5
    /// Hellos in one round: one every `helloTimeout` seconds until the Mac answers.
    static let helloAttempts = 3
    /// When a whole round went unanswered, the next one starts after the first of these delays;
    /// each further failed round waits for the next, the last one repeating. A Mac that was only
    /// busy, or a link that was only bad for a while, gets another chance without the phone
    /// pestering one that never answers.
    static let helloRetryDelays: [TimeInterval] = [30, 60, 120]
    /// How many of the latest hellos still count when their ack arrives: a slow Mac's answer to an
    /// earlier attempt completes the handshake just as well as the newest one's.
    static let helloNoncesKept = 8

    /// Sends `hello` when the connection is subscribed, the Mac is online and no handshake is
    /// running or done for the Mac's current epoch.
    func evaluateHello(_ macId: String) async {
        guard subscribed, case .connected = phase, let mac = macs[macId], mac.online else { return }
        switch mac.sync {
        case .none:
            await sendHello(macId, attempt: 1)
        case .helloSent:
            break
        case .acked, .synced:
            // The Mac restarted: its epoch changed, so everything we hold is stale.
            if let epoch = mac.presence?.e, epoch != mac.epoch { await sendHello(macId, attempt: 1) }
        }
    }

    /// Starts the handshake over for a Mac that is online but never completed it, without waiting
    /// for the next scheduled attempt: the user pulled to refresh, or a notification showed that
    /// the phone and the Mac are out of step. Does nothing for a Mac that answered already.
    func restartHello(_ macId: String) async {
        guard subscribed, case .connected = phase, let mac = macs[macId], mac.online, !mac.sync.isHandshaken else { return }
        macs[macId]?.helloRounds = 0
        await sendHello(macId, attempt: 1)
    }

    func sendHello(_ macId: String, attempt: Int) async {
        guard var mac = macs[macId] else { return }
        let nonce = Bytes.newMsgId()
        mac.sync = .helloSent(nonce: nonce, attempt: attempt)
        mac.helloNonces.append(nonce)
        if mac.helloNonces.count > Self.helloNoncesKept {
            mac.helloNonces.removeFirst(mac.helloNonces.count - Self.helloNoncesKept)
        }
        macs[macId] = mac
        // A slow retry that was waiting is replaced by this attempt.
        cancelTimer("hello.retry." + macId)
        let hello = Hello(nonce: nonce, appVer: config.appVersion, deviceName: config.deviceName)
        await send(hello, to: macId)
        after("hello." + macId, Self.helloTimeout) { engine in await engine.helloTimedOut(macId, nonce: nonce) }
    }

    func helloTimedOut(_ macId: String, nonce: String) async {
        guard let mac = macs[macId], case .helloSent(let pending, let attempt) = mac.sync, pending == nonce else { return }
        if attempt < Self.helloAttempts {
            await sendHello(macId, attempt: attempt + 1)
        } else {
            // The Mac did not answer this round. Its presence changing or the connection being renewed
            // starts the handshake again at once; failing that, a slower round follows by itself, and
            // an answer to any hello of the earlier rounds is still accepted when it arrives.
            macs[macId]?.sync = .none
            scheduleHelloRetry(macId)
        }
        publish()
    }

    /// Plans the next round of hellos after one went unanswered.
    func scheduleHelloRetry(_ macId: String) {
        guard var mac = macs[macId] else { return }
        let delays = Self.helloRetryDelays
        let delay = delays[min(mac.helloRounds, delays.count - 1)]
        mac.helloRounds += 1
        macs[macId] = mac
        after("hello.retry." + macId, delay) { engine in await engine.evaluateHello(macId) }
    }

    func onHelloAck(_ macId: String, _ ack: HelloAck) async {
        guard var mac = macs[macId], mac.awaitsHelloAck(nonce: ack.nonce) else { return }
        cancelTimer("hello." + macId)
        cancelTimer("hello.retry." + macId)
        mac.helloNonces = []
        mac.helloRounds = 0
        mac.sync = .acked
        mac.epoch = ack.epoch
        mac.caps = ack.caps
        mac.slot = ack.slot
        mac.inboxRev = nil
        mac.slotRev = nil
        mac.agents = [:]
        mac.agentsRev = nil
        mac.followRev = nil
        if !ack.macName.isEmpty, ack.macName != mac.record.macName {
            mac.record.macName = ack.macName
            try? vault.savePairing(mac.record)
        }
        macs[macId] = mac
        var meta = store.meta(macId: macId)
        meta.lastEpoch = ack.epoch
        meta.lastSeenAt = clock.now()
        store.saveMeta(meta, macId: macId)
        // Every session starts with the Mac getting the handle again: a message sent while the link was
        // going down, or dropped on the way, is otherwise lost for good, and nothing tells the phone.
        pushSent[macId] = nil
        await sendPushHandle(to: macId)
        // The Mac forgets who watches its todos with the connection.
        await reopenTodoWatch(macId)
    }

    func onMacInfo(_ macId: String, _ info: MacInfo) {
        guard !info.macName.isEmpty, var mac = macs[macId] else { return }
        mac.record.macName = info.macName
        macs[macId] = mac
        try? vault.savePairing(mac.record)
    }

    func onSlotState(_ macId: String, _ state: SlotState) {
        guard var mac = macs[macId], mac.epoch != nil else { return }
        if let last = mac.slotRev, state.rev <= last { return }
        mac.slotRev = state.rev
        mac.slot = SlotInfo(active: state.active, autoActivation: state.autoActivation)
        macs[macId] = mac
    }

    func onError(_ macId: String, _ error: ErrorBody) {
        switch error.code {
        case "clock_skew":
            clockSkew = true
        case "not_paired":
            // The Mac forgot this phone (it was revoked there while we were away).
            Task { await self.onUnpaired(macId) }
        default:
            break
        }
    }

    func requestResync(_ macId: String, stream: String) async {
        await send(Resync(stream: stream), to: macId)
    }

    /// The Mac revoked this phone, or the user removed the pairing here.
    func onUnpaired(_ macId: String) async {
        guard let mac = macs[macId] else { return }
        removeMac(macId)
        emit(.macUnpaired(macId: macId, macName: mac.record.displayName))
        publish()
    }

    /// Forgets a Mac: keys, cached requests, outbox, notes and history.
    func removeMac(_ macId: String) {
        failTransfers(for: macId, with: .unknownMac)
        failInterjects(for: macId, with: .unknownMac)
        failTodos(for: macId, with: .unknownMac)
        wantedTodoWatches.remove(macId)
        dropWatches(of: macId, forget: true)
        interruptTasks(of: macId, forget: true)
        taskOptions[macId] = nil
        taskTodos = taskTodos.filter { !$0.key.hasPrefix(macId + "|") }
        followRequests[macId] = nil
        cancelTimer("hello." + macId)
        cancelTimer("hello.retry." + macId)
        lastRepairAt[macId] = nil
        vault.removePairing(macId: macId)
        store.forget(macId: macId)
        for key in requests.keys where requests[key]?.cached.macId == macId {
            store.removeDraft(key: key)
            requests[key] = nil
        }
        for (id, item) in outbox where item.macId == macId {
            outbox[id] = nil
            cancelTimer("ack." + id)
        }
        pushSent[macId] = nil
        macs[macId] = nil
        historyRevision += 1
        if macs.isEmpty, pairingFlow == nil {
            broker.disconnect()
            resetConnectionState()
            phase = .idle
        }
    }

    // MARK: renaming

    public func renameMac(macId: String, alias: String?) async {
        guard var mac = macs[macId] else { return }
        let trimmed = alias?.trimmingCharacters(in: .whitespacesAndNewlines)
        mac.record.alias = (trimmed?.isEmpty ?? true) ? nil : trimmed
        macs[macId] = mac
        try? vault.savePairing(mac.record)
        publish()
    }

    public func setDeviceName(_ name: String) async {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        config.deviceName = String(trimmed.prefix(60))
    }

    public func unpair(macId: String) async {
        guard macs[macId] != nil else { return }
        if macs[macId]?.sync == .synced { await send(Unpair(), to: macId) }
        removeMac(macId)
        publish()
    }

    /// Back to the first-launch state: every pairing, the broker account and the push handle are
    /// gone (the history of answered requests stays until it is cleared).
    public func forgetAll() async {
        if pairingFlow != nil { await cancelPairing() }
        for macId in Array(macs.keys) {
            if macs[macId]?.sync == .synced { await send(Unpair(), to: macId) }
            removeMac(macId)
        }
        vault.broker = nil
        vault.push = nil
        settings = nil
        pushSent = [:]
        pushStatus = deviceToken == nil ? .noToken : .noRelay
        terminalPairing = nil
        broker.disconnect()
        resetConnectionState()
        phase = .idle
        publish()
    }
}
