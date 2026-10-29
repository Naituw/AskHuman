import Foundation

/// An in-memory MQTT broker with the behavior the protocol depends on: retained messages, last
/// wills, `+` wildcards and a per-user topic ACL (`ah/<username>/#`). Used by the tests of the
/// session engine and by the mock backend; never talks to the network.
public final class LoopbackBroker: @unchecked Sendable {
    private let lock = NSLock()
    private var accounts: [String: String]
    private var retained: [String: Data] = [:]
    private var clients: [ObjectIdentifier: LoopbackClient] = [:]
    /// Messages published by anyone, in order (tests inspect them).
    public private(set) var log: [(topic: String, payload: Data, retained: Bool)] = []

    public init(accounts: [String: String]) { self.accounts = accounts }

    public func makeClient() -> LoopbackClient { LoopbackClient(broker: self) }

    // MARK: used by clients

    fileprivate func authenticate(_ user: String, _ password: String) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return accounts[user] == password
    }

    fileprivate func register(_ client: LoopbackClient) {
        lock.lock(); defer { lock.unlock() }
        clients[ObjectIdentifier(client)] = client
    }

    fileprivate func unregister(_ client: LoopbackClient) {
        lock.lock(); defer { lock.unlock() }
        clients[ObjectIdentifier(client)] = nil
    }

    fileprivate func publish(from sender: LoopbackClient?, user: String, topic: String, payload: Data, retain: Bool) -> Bool {
        guard topic.hasPrefix("ah/\(user)/") else { return false }
        lock.lock()
        log.append((topic, payload, retain))
        if retain {
            if payload.isEmpty { retained[topic] = nil } else { retained[topic] = payload }
        }
        let targets = Array(clients.values)
        lock.unlock()
        for target in targets {
            target.deliver(topic: topic, payload: payload, retained: false)
        }
        return true
    }

    fileprivate func retainedMatching(_ filter: String) -> [(String, Data)] {
        lock.lock(); defer { lock.unlock() }
        return retained.filter { Self.matches(filter: filter, topic: $0.key) }.map { ($0.key, $0.value) }.sorted { $0.0 < $1.0 }
    }

    fileprivate func allowsSubscribe(user: String, filter: String) -> Bool { filter.hasPrefix("ah/\(user)/") }

    /// Retained payload of a topic (for assertions).
    public func retainedPayload(_ topic: String) -> Data? {
        lock.lock(); defer { lock.unlock() }
        return retained[topic]
    }

    static func matches(filter: String, topic: String) -> Bool {
        let f = filter.split(separator: "/", omittingEmptySubsequences: false)
        let t = topic.split(separator: "/", omittingEmptySubsequences: false)
        var i = 0
        while i < f.count {
            if f[i] == "#" { return true }
            guard i < t.count else { return false }
            if f[i] != "+" && f[i] != t[i] { return false }
            i += 1
        }
        return f.count == t.count
    }
}

/// One client of a `LoopbackBroker`.
public final class LoopbackClient: BrokerClient, @unchecked Sendable {
    public let events: AsyncStream<BrokerEvent>
    private let continuation: AsyncStream<BrokerEvent>.Continuation
    private let broker: LoopbackBroker
    private let lock = NSLock()
    private var user = ""
    private var will: BrokerWill?
    private var filters: Set<String> = []
    private var connected = false
    private var via: BrokerEndpoint.Kind = .tls
    /// Endpoint kinds the test broker refuses (to exercise the TLS → WSS fallback).
    public var refusedKinds: Set<BrokerEndpoint.Kind> = []
    private var publishesFail = false
    private var inboundDead = false
    private var publishHook: (@Sendable (String) async -> Void)?
    private var attemptLog: [BrokerEndpoint.Kind] = []

    /// A link that takes nothing: publishes return `false` while the client still looks connected, as
    /// in the moments before a dropped connection is noticed.
    public var failsPublishes: Bool {
        get { lock.lock(); defer { lock.unlock() }; return publishesFail }
        set { lock.lock(); publishesFail = newValue; lock.unlock() }
    }

    /// A link that brings nothing in: publishes go out and are acknowledged, but nothing is delivered
    /// to this client, as on a connection that died on the way in without anyone noticing. A new
    /// connection is a healthy one again.
    public var dropsInbound: Bool {
        get { lock.lock(); defer { lock.unlock() }; return inboundDead }
        set { lock.lock(); inboundDead = newValue; lock.unlock() }
    }

    /// Awaited with the topic before every publish: a test can hold a message on its way with it.
    public var beforePublish: (@Sendable (String) async -> Void)? {
        get { lock.lock(); defer { lock.unlock() }; return publishHook }
        set { lock.lock(); publishHook = newValue; lock.unlock() }
    }

    /// Endpoint kind of every connection attempt, in order (tests inspect them).
    public var attempts: [BrokerEndpoint.Kind] {
        lock.lock(); defer { lock.unlock() }
        return attemptLog
    }

    fileprivate init(broker: LoopbackBroker) {
        self.broker = broker
        var cont: AsyncStream<BrokerEvent>.Continuation!
        events = AsyncStream(bufferingPolicy: .unbounded) { cont = $0 }
        continuation = cont
    }

    public var isConnected: Bool {
        lock.lock(); defer { lock.unlock() }
        return connected
    }

    public func connect(endpoint: BrokerEndpoint, params: BrokerConnectParams) {
        lock.lock()
        attemptLog.append(endpoint.kind)
        lock.unlock()
        if refusedKinds.contains(endpoint.kind) {
            continuation.yield(.disconnected(reason: "port unreachable", authFailed: false, requested: false))
            return
        }
        guard broker.authenticate(params.username, params.password) else {
            continuation.yield(.disconnected(reason: "bad credentials", authFailed: true, requested: false))
            return
        }
        lock.lock()
        user = params.username
        will = params.will
        filters = []
        connected = true
        inboundDead = false
        via = endpoint.kind
        lock.unlock()
        broker.register(self)
        continuation.yield(.connected(via: endpoint.kind))
    }

    public func disconnect() {
        lock.lock()
        let was = connected
        connected = false
        will = nil
        filters = []
        lock.unlock()
        broker.unregister(self)
        if was { continuation.yield(.disconnected(reason: "closed", authFailed: false, requested: true)) }
    }

    /// Simulates a dropped connection: the broker publishes the last will.
    public func drop() {
        lock.lock()
        let was = connected
        let lastWill = will
        let owner = user
        connected = false
        will = nil
        filters = []
        lock.unlock()
        guard was else { return }
        broker.unregister(self)
        if let lastWill { _ = broker.publish(from: nil, user: owner, topic: lastWill.topic, payload: lastWill.payload, retain: true) }
        continuation.yield(.disconnected(reason: "connection lost", authFailed: false, requested: false))
    }

    public func subscribe(_ topics: [String]) {
        lock.lock()
        let owner = user
        let live = connected
        lock.unlock()
        guard live else { return }
        var ok: [String] = []
        var failed: [String] = []
        for topic in topics {
            if broker.allowsSubscribe(user: owner, filter: topic) { ok.append(topic) } else { failed.append(topic) }
        }
        lock.lock()
        filters.formUnion(ok)
        lock.unlock()
        continuation.yield(.subscribed(topics: ok, failed: failed))
        for topic in ok {
            for (t, payload) in broker.retainedMatching(topic) {
                continuation.yield(.message(topic: t, payload: payload, retained: true))
            }
        }
    }

    public func unsubscribe(_ topics: [String]) {
        lock.lock(); defer { lock.unlock() }
        filters.subtract(topics)
    }

    private func session() -> (user: String, live: Bool) {
        lock.lock(); defer { lock.unlock() }
        return (user, connected)
    }

    public func publish(topic: String, payload: Data, retain: Bool) async -> Bool {
        if let hook = beforePublish { await hook(topic) }
        let (owner, live) = session()
        guard live, !failsPublishes else { return false }
        return broker.publish(from: self, user: owner, topic: topic, payload: payload, retain: retain)
    }

    fileprivate func deliver(topic: String, payload: Data, retained: Bool) {
        lock.lock()
        let wanted = connected && !inboundDead && filters.contains { LoopbackBroker.matches(filter: $0, topic: topic) }
        lock.unlock()
        if wanted { continuation.yield(.message(topic: topic, payload: payload, retained: retained)) }
    }
}
