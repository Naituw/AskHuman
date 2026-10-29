import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The real MQTT client, with two things a test can see and do: how many connections the engine opened,
/// and a switch that makes the current connection deaf (everything it receives is thrown away, while
/// publishes and pings carry on and are acknowledged), as one that died on the way in would be.
final class ObservedBroker: BrokerClient, @unchecked Sendable {
    private let inner = MQTTBrokerClient()
    private let lock = NSLock()
    private var connects = 0
    private var deaf = false
    let events: AsyncStream<BrokerEvent>

    init() {
        let (stream, continuation) = AsyncStream<BrokerEvent>.makeStream(bufferingPolicy: .unbounded)
        events = stream
        let innerEvents = inner.events
        Task { [weak self] in
            for await event in innerEvents {
                guard let self else { break }
                if case .message = event, self.isDeaf { continue }
                continuation.yield(event)
            }
            continuation.finish()
        }
    }

    var connectCount: Int { lock.withLock { connects } }
    var isDeaf: Bool { lock.withLock { deaf } }
    /// Takes effect for the current connection only: a new one is healthy.
    func goDeaf() { lock.withLock { deaf = true } }

    func connect(endpoint: BrokerEndpoint, params: BrokerConnectParams) {
        lock.withLock {
            connects += 1
            deaf = false
        }
        inner.connect(endpoint: endpoint, params: params)
    }

    func disconnect() { inner.disconnect() }
    func subscribe(_ topics: [String]) { inner.subscribe(topics) }
    func unsubscribe(_ topics: [String]) { inner.unsubscribe(topics) }
    func publish(topic: String, payload: Data, retain: Bool) async -> Bool {
        await inner.publish(topic: topic, payload: payload, retain: retain)
    }
}

/// The phone's link check (`PhoneEngine.checkLink`) against the real gateway in a real daemon, over a
/// real MQTT client and broker: the part the in-process tests cannot say anything about is whether a
/// message the phone publishes to its own down topic comes back through them.
///
///   AH_E2E_BIN=<installed AskHuman> AH_E2E_BROKER_DIR=ios/Tools/local-broker swift test --filter LinkCheckE2ETests
final class LinkCheckE2ETests: E2ECase {
    func testTheProbeComesBackAndAHealthyConnectionIsKept() async throws {
        let observed = ObservedBroker()
        phone = E2EPhone(bundleId: environment.phoneBundleId, broker: observed)
        try await pair()
        let macId = try unwrap(await phone.state().macs.first?.id)
        XCTAssertEqual(observed.connectCount, 1)

        await phone.engine.checkLink(macId: macId)
        // Back long before the time limit: the token is cleared by the echo, not by the timer.
        await eventually("the probe to come back", timeout: PhoneEngine.probeTimeout - 1) {
            await self.phone.engine.probeToken == nil
        }
        // Past the time limit nothing has happened: the connection was healthy and stays.
        try await Task.sleep(nanoseconds: UInt64((PhoneEngine.probeTimeout + 1) * 1_000_000_000))
        XCTAssertEqual(observed.connectCount, 1, "a healthy connection must not be replaced")
        let link = await phone.state().macs.first?.link
        XCTAssertEqual(link, .online)
    }

    func testADeafConnectionIsReplacedAndTheMissedRequestArrives() async throws {
        let observed = ObservedBroker()
        phone = E2EPhone(bundleId: environment.phoneBundleId, broker: observed)
        try await pair()
        let macId = try unwrap(await phone.state().macs.first?.id)

        // The connection goes deaf. The Mac believes the phone is looking and sends the request live.
        observed.goDeaf()
        let ask = try mac.ask(["-q", "Did it get through?", "-o", "Yes", "-o", "No"])
        try await Task.sleep(nanoseconds: 1_500_000_000)
        let inboxBefore = await phone.state().inbox
        XCTAssertTrue(inboxBefore.isEmpty, "a deaf connection brings nothing in")

        // A notification about it reaches the open app.
        await phone.engine.checkLink(macId: macId)
        await eventually("a new connection", timeout: PhoneEngine.probeTimeout + 6) { observed.connectCount == 2 }
        let item = try await phone.request()
        XCTAssertEqual(item.view?.ask?.questions.first?.text, "Did it get through?")

        _ = try await phone.engine.submitAnswer(macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0])])
        await assertFinishes(ask)
        XCTAssertTrue(ask.stdout.contains("Yes"), ask.stdout)
    }
}
