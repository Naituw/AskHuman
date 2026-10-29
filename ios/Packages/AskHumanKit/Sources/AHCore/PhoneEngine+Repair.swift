import Foundation
import AHProtocol

// Self-repair for a phone that is out of step with a Mac although its link looks fine.
//
// A Mac pushes a request only when it sent it live and heard no acknowledgement, or when it believed
// nobody was looking. So a push that reaches the open app for a request the inbox does not hold means
// something between the two is broken, and two things can be: the broker connection can look healthy
// for a long while after it stopped carrying anything, and a handshake can stay unfinished although
// the Mac already treats the phone as live. `checkLink` tests the first and mends the second.

extension PhoneEngine {
    /// How long the phone's own probe may take to come back through the broker before the
    /// connection is declared dead and replaced.
    static let probeTimeout: TimeInterval = 4
    /// A Mac's notifications make the engine look into its session at most this often.
    static let repairCooldown: TimeInterval = 10

    public func checkLink(macId: String) async {
        guard foreground, macs[macId] != nil else { return }
        let now = clock.now()
        if let last = lastRepairAt[macId], now.timeIntervalSince(last) < Self.repairCooldown { return }
        lastRepairAt[macId] = now
        switch phase {
        case .backoff, .refused:
            // Known to be down: no reason to sit out the back-off.
            phase = .idle
            backoff.reset()
            startConnecting()
        case .idle:
            startConnecting()
        case .connecting:
            // Already on its way up; the handshake and the snapshot that follow bring the request.
            break
        case .connected:
            guard subscribed else { return }
            probeLink()
            await mendSession(macId)
        }
        publish()
    }

    /// Brings the session with one Mac back in step: the handshake again when it never finished (a
    /// snapshot would be ignored without it), the inbox again when it did.
    func mendSession(_ macId: String) async {
        guard let mac = macs[macId], mac.online else { return }
        if mac.sync.isHandshaken {
            await requestResync(macId, stream: "inbox")
        } else {
            await restartHello(macId)
        }
    }

    /// Publishes a token to the phone's own down topic, which the broker hands straight back when the
    /// connection carries messages both ways. Only the echo proves that: an acknowledgement of the
    /// publish says nothing about what comes in, and traffic from a Mac nothing about what goes out.
    func probeLink() {
        guard let active = activeSettings else { return }
        let token = Bytes.random(16)
        probeToken = token
        after("probe", Self.probeTimeout) { engine in await engine.probeTimedOut(token) }
        count(sent: token.count)
        let topic = Wire.topicDown(user: active.username, phoneId: vault.phoneId())
        // Publishing waits for the broker's acknowledgement, and a dead connection never gives one
        // (the call gives up after ten seconds): the timer above must not wait for it.
        Task { [broker] in
            if await broker.publish(topic: topic, payload: token, retain: false) == false {
                await self.probeTimedOut(token)
            }
        }
    }

    func probeTimedOut(_ token: Data) {
        // The echo came back (it cleared the token), or a newer probe or a new connection took over.
        guard probeToken == token, foreground, case .connected = phase else { return }
        probeToken = nil
        reconnectNow()
    }

    /// Drops the connection and builds a new one at once, without back-off: the way out of one that
    /// looks up but carries nothing. What the session learned starts over, as after any reconnect.
    func reconnectNow() {
        cancelTimer("connect")
        cancelTimer("reconnect")
        broker.disconnect()
        resetConnectionState()
        phase = .idle
        backoff.reset()
        startConnecting()
        publish()
    }
}
