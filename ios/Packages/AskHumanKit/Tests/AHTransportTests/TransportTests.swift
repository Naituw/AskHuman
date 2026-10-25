import XCTest
@testable import AHTransport

final class TransportTests: XCTestCase {
    // MARK: endpoint plan

    func testTLSIsTriedBeforeWebSocketByDefault() {
        let plan = EndpointPlan.endpoints(host: "mq.example.com", port: 8883, wsPort: 8084, lastTransport: nil, allowPlainLoopback: false)
        XCTAssertEqual(plan, [
            BrokerEndpoint(kind: .tls, host: "mq.example.com", port: 8883),
            BrokerEndpoint(kind: .wss, host: "mq.example.com", port: 8084),
        ])
    }

    func testTheTransportThatWorkedLastComesFirst() {
        let plan = EndpointPlan.endpoints(host: "mq.example.com", port: 8883, wsPort: 8084, lastTransport: "wss", allowPlainLoopback: false)
        XCTAssertEqual(plan.map(\.kind), [.wss, .tls])
        let tls = EndpointPlan.endpoints(host: "mq.example.com", port: 8883, wsPort: 8084, lastTransport: "tls", allowPlainLoopback: false)
        XCTAssertEqual(tls.map(\.kind), [.tls, .wss])
    }

    func testWithoutAWebSocketPortOnlyTLSIsUsed() {
        let plan = EndpointPlan.endpoints(host: "mq.example.com", port: 8883, wsPort: 0, lastTransport: "wss", allowPlainLoopback: false)
        XCTAssertEqual(plan.map(\.kind), [.tls])
    }

    func testPlainTCPIsOnlyForLoopbackHostsWhenAllowed() {
        for host in ["localhost", "127.0.0.1", "::1", "LOCALHOST"] {
            let plan = EndpointPlan.endpoints(host: host, port: 1883, wsPort: 8084, lastTransport: nil, allowPlainLoopback: true)
            XCTAssertEqual(plan, [BrokerEndpoint(kind: .tcp, host: host, port: 1883)], host)
        }
        let remote = EndpointPlan.endpoints(host: "mq.example.com", port: 1883, wsPort: 8084, lastTransport: nil, allowPlainLoopback: true)
        XCTAssertFalse(remote.contains { $0.kind == .tcp }, "never plain TCP to a remote broker")
        let release = EndpointPlan.endpoints(host: "localhost", port: 8883, wsPort: 8084, lastTransport: nil, allowPlainLoopback: false)
        XCTAssertFalse(release.contains { $0.kind == .tcp }, "release builds never allow it")
    }

    // MARK: back-off

    func testBackoffStepsUpAndStaysAtThirtySeconds() {
        var backoff = ReconnectBackoff()
        let delays = (0..<9).map { _ in backoff.next(jitter: 0) }
        XCTAssertEqual(delays, [1, 2, 4, 8, 15, 30, 30, 30, 30])
        backoff.reset()
        XCTAssertEqual(backoff.next(jitter: 0), 1)
    }

    func testBackoffJitterAddsUpToTwentyPercent() {
        var backoff = ReconnectBackoff()
        for _ in 0..<200 {
            backoff.reset()
            let delay = backoff.next()
            XCTAssertGreaterThanOrEqual(delay, 1)
            XCTAssertLessThanOrEqual(delay, 1.2 + 1e-9)
        }
    }

    // MARK: token bucket

    func testTokenBucketAllowsABurstThenPaces() {
        var bucket = TokenBucket(rate: 6, burst: 10, now: 100)
        for _ in 0..<10 { XCTAssertEqual(bucket.take(now: 100), 0) }
        let wait = bucket.take(now: 100)
        XCTAssertEqual(wait, 1.0 / 6.0, accuracy: 1e-9)
        let second = bucket.take(now: 100)
        XCTAssertEqual(second, 2.0 / 6.0, accuracy: 1e-9, "waits accumulate for queued messages")
    }

    func testTokenBucketRefillsWithTimeButNotBeyondTheBurst() {
        var bucket = TokenBucket(rate: 6, burst: 10, now: 0)
        for _ in 0..<10 { _ = bucket.take(now: 0) }
        XCTAssertEqual(bucket.take(now: 1), 0, "six tokens came back after a second")
        for _ in 0..<5 { XCTAssertEqual(bucket.take(now: 1), 0) }
        XCTAssertGreaterThan(bucket.take(now: 1), 0)
        // After a long pause the bucket holds the burst, not more.
        for _ in 0..<10 { XCTAssertEqual(bucket.take(now: 1000), 0) }
        XCTAssertGreaterThan(bucket.take(now: 1000), 0)
    }

    // MARK: loopback broker

    private func connect(_ broker: LoopbackBroker, user: String = "u1", password: String = "pw", will: BrokerWill? = nil) -> LoopbackClient {
        let client = broker.makeClient()
        client.connect(
            endpoint: BrokerEndpoint(kind: .tls, host: "h", port: 1),
            params: BrokerConnectParams(clientId: UUID().uuidString, username: user, password: password, will: will)
        )
        return client
    }

    private func next(_ client: LoopbackClient, timeout: TimeInterval = 2) async -> BrokerEvent? {
        await withTaskGroup(of: BrokerEvent?.self) { group in
            group.addTask {
                var iterator = client.events.makeAsyncIterator()
                return await iterator.next()
            }
            group.addTask {
                try? await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
                return nil
            }
            let first = await group.next() ?? nil
            group.cancelAll()
            return first
        }
    }

    func testBadCredentialsAreReportedAsAuthFailure() async {
        let broker = LoopbackBroker(accounts: ["u1": "pw"])
        let client = connect(broker, password: "wrong")
        let event = await next(client)
        XCTAssertEqual(event, .disconnected(reason: "bad credentials", authFailed: true, requested: false))
        XCTAssertFalse(client.isConnected)
    }

    func testTopicsOutsideTheUsersNamespaceAreRefused() async {
        let broker = LoopbackBroker(accounts: ["u1": "pw"])
        let client = connect(broker)
        _ = await next(client) // connected
        client.subscribe(["ah/u1/d/x", "ah/other/d/x"])
        let event = await next(client)
        XCTAssertEqual(event, .subscribed(topics: ["ah/u1/d/x"], failed: ["ah/other/d/x"]))
        let published = await client.publish(topic: "ah/other/u/x", payload: Data([1]), retain: false)
        XCTAssertFalse(published)
    }

    func testRetainedMessagesAreDeliveredOnSubscribeAndWildcardsMatch() async {
        let broker = LoopbackBroker(accounts: ["u1": "pw"])
        let publisher = connect(broker)
        _ = await publisher.publish(topic: "ah/u1/s/mac1", payload: Data("one".utf8), retain: true)
        _ = await publisher.publish(topic: "ah/u1/s/mac2", payload: Data("two".utf8), retain: true)
        _ = await publisher.publish(topic: "ah/u1/d/phone", payload: Data("down".utf8), retain: true)

        let subscriber = connect(broker)
        _ = await next(subscriber) // connected
        subscriber.subscribe(["ah/u1/s/+"])
        var topics: [String] = []
        _ = await next(subscriber) // suback
        for _ in 0..<2 {
            if case .message(let topic, _, let retained) = await next(subscriber) {
                XCTAssertTrue(retained)
                topics.append(topic)
            }
        }
        XCTAssertEqual(topics, ["ah/u1/s/mac1", "ah/u1/s/mac2"])
        XCTAssertNil(broker.retainedPayload("ah/u1/nothing"))
    }

    func testAnEmptyRetainedPayloadClearsTheMessage() async {
        let broker = LoopbackBroker(accounts: ["u1": "pw"])
        let client = connect(broker)
        _ = await client.publish(topic: "ah/u1/s/a", payload: Data([1]), retain: true)
        XCTAssertNotNil(broker.retainedPayload("ah/u1/s/a"))
        _ = await client.publish(topic: "ah/u1/s/a", payload: Data(), retain: true)
        XCTAssertNil(broker.retainedPayload("ah/u1/s/a"))
    }

    func testTheLastWillIsPublishedOnADropButNotOnAGoodbye() async {
        let broker = LoopbackBroker(accounts: ["u1": "pw"])
        let will = BrokerWill(topic: "ah/u1/s/dev", payload: Data("gone".utf8))

        let crashing = connect(broker, will: will)
        crashing.drop()
        XCTAssertEqual(broker.retainedPayload("ah/u1/s/dev"), Data("gone".utf8))

        _ = await connect(broker).publish(topic: "ah/u1/s/dev", payload: Data("here".utf8), retain: true)
        let polite = connect(broker, will: will)
        polite.disconnect()
        XCTAssertEqual(broker.retainedPayload("ah/u1/s/dev"), Data("here".utf8), "a clean disconnect sends no will")
    }

    func testEndpointKindsCanBeRefused() async {
        let broker = LoopbackBroker(accounts: ["u1": "pw"])
        let client = broker.makeClient()
        client.refusedKinds = [.tls]
        client.connect(endpoint: BrokerEndpoint(kind: .tls, host: "h", port: 1), params: BrokerConnectParams(clientId: "c", username: "u1", password: "pw"))
        let event = await next(client)
        XCTAssertEqual(event, .disconnected(reason: "port unreachable", authFailed: false, requested: false))
        client.connect(endpoint: BrokerEndpoint(kind: .wss, host: "h", port: 2), params: BrokerConnectParams(clientId: "c", username: "u1", password: "pw"))
        let second = await next(client)
        XCTAssertEqual(second, .connected(via: .wss))
        XCTAssertEqual(client.attempts, [.tls, .wss])
    }
}
