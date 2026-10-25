import Foundation
import AHProtocol
import AHStore
import AHTransport

/// Everything of one pairing attempt (plan §3.3). The phone derives the same keys and check code
/// as the Mac from the pairing code and its own ephemeral key.
struct PairingFlow {
    var code: PairCode
    var settings: BrokerSettings
    let keyPair: AHCrypto.KeyPair
    let ticketKey: Data
    let keys: AHCrypto.PairKeys
    let sas: String
    var stage: PairingStage
    var requestSent = false
    let expiresAt: Date
    let pairTopic: String

    var macId: String { code.m }
}

extension PhoneEngine {
    // MARK: state

    func pairingState() -> PairingState? {
        guard let flow = pairingFlow else { return terminalPairing }
        return PairingState(stage: flow.stage, macName: flow.code.mn, sas: flow.sas, expiresAt: flow.expiresAt)
    }

    // MARK: start

    public func startPairing(code text: String) async {
        terminalPairing = nil
        guard let code = PairCode.decode(text), code.v == Wire.proto, Bytes.isValidId(code.m), Bytes.isValidId(code.t),
              Wire.isValidBrokerUser(code.u), !code.h.isEmpty,
              let secret = Bytes.b64uDecode(code.ts), secret.count == 32,
              let macPublic = Bytes.b64uDecode(code.ek), macPublic.count == 32
        else {
            return failImmediately(.invalidCode, macName: "")
        }
        let now = clock.now()
        let expiresAt = Date(timeIntervalSince1970: TimeInterval(code.x))
        guard expiresAt > now else { return failImmediately(.expired, macName: code.mn) }

        let incoming = BrokerSettings(code: code)
        if let current = settings, !macs.isEmpty, !current.sameAccount(as: incoming) {
            return failImmediately(.differentBroker, macName: code.mn)
        }
        var effective = incoming
        if let current = settings, current.sameAccount(as: incoming) { effective.lastTransport = current.lastTransport }

        let keyPair = AHCrypto.KeyPair()
        guard let shared = keyPair.sharedSecret(with: macPublic) else { return failImmediately(.invalidCode, macName: code.mn) }
        let phoneId = vault.phoneId()
        let keys = AHCrypto.derivePairKeys(
            sharedSecret: shared,
            context: AHCrypto.PairContext(
                macId: code.m, phoneId: phoneId, ticketId: code.t, ticketSecret: secret, macPublic: macPublic,
                phonePublic: keyPair.publicKey
            )
        )
        let flow = PairingFlow(
            code: code, settings: effective, keyPair: keyPair, ticketKey: AHCrypto.ticketKey(ticketSecret: secret, ticketId: code.t),
            keys: keys, sas: AHCrypto.sasCode(kSas: keys.sas), stage: .connecting, expiresAt: expiresAt,
            pairTopic: Wire.topicPair(user: code.u, ticketId: code.t)
        )

        let differentAccount: Bool = {
            guard let active = activeSettings else { return false }
            return !active.sameAccount(as: effective) || active.password != effective.password
        }()
        pairingFlow = flow
        foreground = true
        let remaining = expiresAt.timeIntervalSince(now)
        after("pairing", remaining) { engine in await engine.pairingExpired() }

        if case .connected = phase, !differentAccount {
            // Already on the right broker account: just listen on the ticket topic.
            pendingSubscriptions.insert(flow.pairTopic)
            broker.subscribe([flow.pairTopic])
        } else {
            broker.disconnect()
            resetConnectionState()
            phase = .idle
            startConnecting()
        }
        publish()
    }

    func failImmediately(_ failure: PairingFailure, macName: String) {
        terminalPairing = PairingState(stage: .failed(failure), macName: macName, sas: nil, expiresAt: clock.now())
        publish()
    }

    func pairingExpired() {
        guard let flow = pairingFlow else { return }
        if case .finishing = flow.stage { return }
        failPairing(.expired)
        publish()
    }

    // MARK: protocol

    func sendPairRequest() async {
        guard var flow = pairingFlow, !flow.requestSent else { return }
        flow.requestSent = true
        flow.stage = .waitingForMac
        pairingFlow = flow
        let request = PairRequest(
            phoneId: vault.phoneId(), name: config.deviceName, ek: Bytes.b64u(flow.keyPair.publicKey),
            appVer: config.appVersion
        )
        guard let plain = try? Plain(request, nowMs: clock.nowMs()),
              let bytes = try? plain.sealed(key: flow.ticketKey, scope: .topic(flow.pairTopic), sender: vault.phoneId(), kid: Wire.kidTicket)
        else {
            return failPairing(.failed("Could not build the pairing request."))
        }
        count(sent: bytes.count)
        let sent = await broker.publish(topic: flow.pairTopic, payload: bytes, retain: false)
        if !sent { failPairing(.failed("Could not reach the broker.")) }
    }

    /// A message on the ticket topic: `pair.pending` or `pair.reject`, sealed with the ticket key.
    func onPairTopic(payload: Data) async {
        guard let flow = pairingFlow, let env = try? AHEnvelope.parse(payload), env.f == flow.macId, env.k == Wire.kidTicket,
              let plain = try? Plain.open(key: flow.ticketKey, scope: .topic(flow.pairTopic), envelope: env),
              abs(plain.ts - clock.nowMs()) <= Wire.clockSkewMs
        else { return }
        switch plain.t {
        case Wire.T.pairPending:
            if case .waitingForMac = flow.stage { pairingFlow?.stage = .macConfirming }
        case Wire.T.pairReject:
            let reason = (try? plain.body(as: PairReject.self).reason) ?? "rejected"
            failPairing(.rejected(reason))
        default:
            break
        }
    }

    /// Tries a message on the phone's own topic as the pairing's `pair.accept` (keys not stored
    /// yet). Returns `true` when the message was consumed by the pairing.
    func onPairAcceptCandidate(topic: String, payload: Data) async -> Bool {
        guard let flow = pairingFlow, let env = try? AHEnvelope.parse(payload), env.f == flow.macId, env.k == Wire.kidPair,
              let plain = try? Plain.open(key: flow.keys.m2p, scope: .topic(topic), envelope: env),
              plain.t == Wire.T.pairAccept, abs(plain.ts - clock.nowMs()) <= Wire.clockSkewMs,
              let accept = try? plain.body(as: PairAccept.self), accept.macId == flow.macId
        else { return false }
        await completePairing(flow: flow, accept: accept)
        return true
    }

    func completePairing(flow: PairingFlow, accept: PairAccept) async {
        pairingFlow?.stage = .finishing
        publish()
        let phoneId = vault.phoneId()
        var record = PairingRecord(
            macId: flow.macId, macName: accept.macName.isEmpty ? flow.code.mn : accept.macName, alias: macs[flow.macId]?.record.alias,
            phoneId: phoneId, kid: Wire.kidPair, m2p: flow.keys.m2p, p2m: flow.keys.p2m, pairedAt: clock.now(), caps: accept.caps
        )
        if let existing = macs[flow.macId]?.record { record.alias = existing.alias }
        do {
            try vault.savePairing(record)
        } catch {
            return failPairing(.failed("Could not store the pairing keys."))
        }
        var runtime = MacRuntime(record: record, lastSeenAt: clock.now())
        runtime.caps = accept.caps
        runtime.presence = presenceCache[flow.macId]
        macs[flow.macId] = runtime
        settings = flow.settings
        vault.broker = flow.settings

        // Register for push before telling the Mac, so `pair.done` can carry the handle.
        await registerPushIfNeeded()
        var done = PairDone(name: config.deviceName)
        if let registration = vault.push {
            done.handle = registration.handle
            done.env = registration.env
            done.expiresAt = registration.expiresAt
            pushSent[flow.macId] = registration.handle
        }
        let sent = await send(done, to: flow.macId)
        guard sent else {
            removeMac(flow.macId)
            return failPairing(.failed("Could not reach the Mac to finish the pairing."))
        }
        cancelTimer("pairing")
        broker.unsubscribe([flow.pairTopic])
        pairingFlow = nil
        terminalPairing = PairingState(stage: .done(macId: flow.macId), macName: record.macName, sas: flow.sas, expiresAt: flow.expiresAt)
        emit(.pairingFinished(macId: flow.macId))
        publish()
        await evaluateHello(flow.macId)
    }

    func failPairing(_ failure: PairingFailure) {
        guard let flow = pairingFlow else { return }
        cancelTimer("pairing")
        if case .connected = phase { broker.unsubscribe([flow.pairTopic]) }
        // A half-made pairing leaves nothing behind.
        if case .finishing = flow.stage, macs[flow.macId] != nil, vault.pairing(macId: flow.macId)?.pairedAt == macs[flow.macId]?.record.pairedAt {
            removeMac(flow.macId)
        }
        terminalPairing = PairingState(stage: .failed(failure), macName: flow.code.mn, sas: flow.sas, expiresAt: flow.expiresAt)
        pairingFlow = nil
        if macs.isEmpty {
            broker.disconnect()
            resetConnectionState()
            if case .refused = phase {} else { phase = .idle }
        }
        publish()
    }

    public func cancelPairing() async {
        guard pairingFlow != nil else { return }
        let flow = pairingFlow!
        cancelTimer("pairing")
        if case .connected = phase { broker.unsubscribe([flow.pairTopic]) }
        pairingFlow = nil
        terminalPairing = nil
        if macs.isEmpty {
            broker.disconnect()
            resetConnectionState()
            phase = .idle
        }
        publish()
    }

    public func finishPairing() async {
        terminalPairing = nil
        publish()
    }
}
