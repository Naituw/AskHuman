import Foundation

/// How the client reaches the broker.
public struct BrokerEndpoint: Sendable, Equatable {
    public enum Kind: String, Sendable {
        /// MQTT over TLS (8883).
        case tls
        /// MQTT over secure WebSocket (8084); works where raw TLS ports are blocked.
        case wss
        /// Plain TCP; only for a loopback test broker in debug builds.
        case tcp
    }

    public var kind: Kind
    public var host: String
    public var port: Int

    public init(kind: Kind, host: String, port: Int) {
        self.kind = kind
        self.host = host
        self.port = port
    }
}

/// A retained message the broker publishes when the connection dies without a goodbye.
public struct BrokerWill: Sendable, Equatable {
    public var topic: String
    public var payload: Data

    public init(topic: String, payload: Data) {
        self.topic = topic
        self.payload = payload
    }
}

public struct BrokerConnectParams: Sendable, Equatable {
    public var clientId: String
    public var username: String
    public var password: String
    public var keepAliveSecs: UInt16
    public var connectTimeout: TimeInterval
    public var will: BrokerWill?

    public init(
        clientId: String, username: String, password: String, keepAliveSecs: UInt16 = 30,
        connectTimeout: TimeInterval = 8, will: BrokerWill? = nil
    ) {
        self.clientId = clientId
        self.username = username
        self.password = password
        self.keepAliveSecs = keepAliveSecs
        self.connectTimeout = connectTimeout
        self.will = will
    }
}

public enum BrokerEvent: Sendable, Equatable {
    case connected(via: BrokerEndpoint.Kind)
    /// The link is down. `requested` is true when `disconnect()` caused it.
    case disconnected(reason: String, authFailed: Bool, requested: Bool)
    /// Result of a `subscribe` call (the SUBACK).
    case subscribed(topics: [String], failed: [String])
    case message(topic: String, payload: Data, retained: Bool)
}

/// The phone's view of the MQTT broker. One implementation talks MQTT (CocoaMQTT), another is an
/// in-memory broker for tests and previews.
public protocol BrokerClient: AnyObject, Sendable {
    /// Connection, subscription and message events, in order. Single consumer.
    var events: AsyncStream<BrokerEvent> { get }

    /// Starts one connection attempt; the outcome arrives as `.connected` or `.disconnected`.
    func connect(endpoint: BrokerEndpoint, params: BrokerConnectParams)
    /// Closes the connection cleanly (no last-will message).
    func disconnect()
    func subscribe(_ topics: [String])
    func unsubscribe(_ topics: [String])
    /// Publishes at QoS 1 and waits for the broker's acknowledgement. `false` when the message
    /// could not be sent or was not acknowledged in time.
    func publish(topic: String, payload: Data, retain: Bool) async -> Bool
}

/// Order in which endpoints are tried: the transport that worked last first, TLS before WSS
/// otherwise; a plain TCP endpoint replaces both for loopback test brokers.
public enum EndpointPlan {
    public static func endpoints(
        host: String, port: Int, wsPort: Int, lastTransport: String?, allowPlainLoopback: Bool
    ) -> [BrokerEndpoint] {
        let isLoopback = ["localhost", "127.0.0.1", "::1"].contains(host.lowercased())
        if allowPlainLoopback && isLoopback {
            return [BrokerEndpoint(kind: .tcp, host: host, port: port)]
        }
        var list = [BrokerEndpoint(kind: .tls, host: host, port: port)]
        if wsPort > 0 { list.append(BrokerEndpoint(kind: .wss, host: host, port: wsPort)) }
        if lastTransport == BrokerEndpoint.Kind.wss.rawValue, list.count == 2 { list.reverse() }
        return list
    }
}

/// Reconnect delays: 1, 2, 4, 8, 15 and then 30 seconds, with up to 20% jitter.
public struct ReconnectBackoff: Sendable {
    public private(set) var attempt = 0
    private static let steps: [TimeInterval] = [1, 2, 4, 8, 15, 30]

    public init() {}

    public mutating func next(jitter: Double = Double.random(in: 0...0.2)) -> TimeInterval {
        let base = Self.steps[min(attempt, Self.steps.count - 1)]
        attempt += 1
        return base * (1 + jitter)
    }

    public mutating func reset() { attempt = 0 }
}

/// Token bucket for outgoing messages (plan §2: 6 per second, bursts of 10).
public struct TokenBucket: Sendable {
    public let rate: Double
    public let burst: Double
    private var tokens: Double
    private var last: TimeInterval

    public init(rate: Double, burst: Double, now: TimeInterval) {
        self.rate = rate
        self.burst = burst
        tokens = burst
        last = now
    }

    /// Seconds to wait before one more message may go out (0 when it may go now). Consumes the token.
    public mutating func take(now: TimeInterval) -> TimeInterval {
        tokens = min(burst, tokens + (now - last) * rate)
        last = now
        if tokens >= 1 {
            tokens -= 1
            return 0
        }
        let wait = (1 - tokens) / rate
        tokens -= 1
        return wait
    }
}
