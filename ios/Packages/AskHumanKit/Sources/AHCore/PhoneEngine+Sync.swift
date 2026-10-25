import Foundation
import AHProtocol
import AHStore

// Handshake and stream synchronisation (plan §3.5): `hello` once the Mac is online, then the
// Mac's snapshots; deltas are applied only in order and only for the epoch the handshake named.

extension PhoneEngine {
    static let helloTimeout: TimeInterval = 5
    static let helloAttempts = 3

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

    func sendHello(_ macId: String, attempt: Int) async {
        guard var mac = macs[macId] else { return }
        let nonce = Bytes.newMsgId()
        mac.sync = .helloSent(nonce: nonce, attempt: attempt)
        macs[macId] = mac
        let hello = Hello(nonce: nonce, appVer: config.appVersion, deviceName: config.deviceName)
        await send(hello, to: macId)
        after("hello." + macId, Self.helloTimeout) { engine in await engine.helloTimedOut(macId, nonce: nonce) }
    }

    func helloTimedOut(_ macId: String, nonce: String) async {
        guard let mac = macs[macId], case .helloSent(let pending, let attempt) = mac.sync, pending == nonce else { return }
        if attempt < Self.helloAttempts {
            await sendHello(macId, attempt: attempt + 1)
        } else {
            // Give up until the Mac's presence changes or the connection is renewed.
            macs[macId]?.sync = .none
        }
        publish()
    }

    func onHelloAck(_ macId: String, _ ack: HelloAck) async {
        guard var mac = macs[macId], case .helloSent(let nonce, _) = mac.sync, nonce == ack.nonce else { return }
        cancelTimer("hello." + macId)
        mac.sync = .acked
        mac.epoch = ack.epoch
        mac.caps = ack.caps
        mac.slot = ack.slot
        mac.inboxRev = nil
        mac.slotRev = nil
        if !ack.macName.isEmpty, ack.macName != mac.record.macName {
            mac.record.macName = ack.macName
            try? vault.savePairing(mac.record)
        }
        macs[macId] = mac
        var meta = store.meta(macId: macId)
        meta.lastEpoch = ack.epoch
        meta.lastSeenAt = clock.now()
        store.saveMeta(meta, macId: macId)
        await sendPushHandle(to: macId)
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
        cancelTimer("hello." + macId)
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
