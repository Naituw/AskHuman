import Foundation
import CocoaMQTT
import CocoaMQTTWebSocket

/// `BrokerClient` over CocoaMQTT (MQTT 3.1.1, clean session, QoS 1). Every connection attempt
/// builds a fresh client, so callbacks of an abandoned attempt can never leak into the next one.
public final class MQTTBrokerClient: NSObject, BrokerClient, @unchecked Sendable {
    public let events: AsyncStream<BrokerEvent>
    private let continuation: AsyncStream<BrokerEvent>.Continuation
    private let queue = DispatchQueue(label: "ah.transport.mqtt")

    // All state below is touched on `queue` only.
    private var link: Link?
    private var waiters: [UInt16: CheckedContinuation<Bool, Never>] = [:]
    private var closing = false

    public override init() {
        var cont: AsyncStream<BrokerEvent>.Continuation!
        events = AsyncStream(bufferingPolicy: .unbounded) { cont = $0 }
        continuation = cont
        super.init()
    }

    deinit { continuation.finish() }

    // MARK: BrokerClient

    public func connect(endpoint: BrokerEndpoint, params: BrokerConnectParams) {
        queue.async { [self] in
            teardown(announce: false)
            let mqtt: CocoaMQTT
            let port = UInt16(clamping: endpoint.port)
            switch endpoint.kind {
            case .tls, .tcp:
                mqtt = CocoaMQTT(clientID: params.clientId, host: endpoint.host, port: port)
                mqtt.enableSSL = endpoint.kind == .tls
            case .wss:
                let socket = CocoaMQTTWebSocket(uri: "/mqtt")
                mqtt = CocoaMQTT(clientID: params.clientId, host: endpoint.host, port: port, socket: socket)
                mqtt.enableSSL = true
            }
            mqtt.username = params.username
            mqtt.password = params.password
            mqtt.keepAlive = params.keepAliveSecs
            mqtt.cleanSession = true
            mqtt.autoReconnect = false
            if let will = params.will {
                mqtt.willMessage = CocoaMQTTMessage(topic: will.topic, payload: [UInt8](will.payload), qos: .qos1, retained: true)
            }
            let link = Link(owner: self, mqtt: mqtt, kind: endpoint.kind)
            mqtt.delegate = link
            mqtt.delegateQueue = queue
            self.link = link
            closing = false
            if !mqtt.connect(timeout: params.connectTimeout) {
                self.link = nil
                continuation.yield(.disconnected(reason: "invalid connection settings", authFailed: false, requested: false))
            }
        }
    }

    public func disconnect() {
        queue.async { [self] in teardown(announce: true) }
    }

    public func subscribe(_ topics: [String]) {
        guard !topics.isEmpty else { return }
        queue.async { [self] in
            link?.mqtt.subscribe(topics.map { ($0, CocoaMQTTQoS.qos1) })
        }
    }

    public func unsubscribe(_ topics: [String]) {
        guard !topics.isEmpty else { return }
        queue.async { [self] in link?.mqtt.unsubscribe(topics) }
    }

    public func publish(topic: String, payload: Data, retain: Bool) async -> Bool {
        await withCheckedContinuation { (cont: CheckedContinuation<Bool, Never>) in
            queue.async { [self] in
                guard let link, link.connected else { return cont.resume(returning: false) }
                let message = CocoaMQTTMessage(topic: topic, payload: [UInt8](payload), qos: .qos1, retained: retain)
                let id = link.mqtt.publish(message)
                guard id > 0 else { return cont.resume(returning: false) }
                let key = UInt16(truncatingIfNeeded: id)
                waiters[key] = cont
                queue.asyncAfter(deadline: .now() + 10) { [weak self] in
                    // No PUBACK in time: report failure (the message may still arrive).
                    self?.waiters.removeValue(forKey: key)?.resume(returning: false)
                }
            }
        }
    }

    // MARK: internals (on `queue`)

    private func teardown(announce: Bool) {
        guard let link else { return }
        self.link = nil
        closing = true
        link.mqtt.delegate = nil
        let wasConnected = link.connected
        link.mqtt.disconnect()
        for (_, waiter) in waiters { waiter.resume(returning: false) }
        waiters.removeAll()
        if announce && wasConnected {
            continuation.yield(.disconnected(reason: "closed", authFailed: false, requested: true))
        }
    }

    fileprivate func connectAck(_ link: Link, _ ack: CocoaMQTTConnAck) {
        guard link === self.link else { return }
        if ack == .accept {
            link.connected = true
            continuation.yield(.connected(via: link.kind))
        } else {
            let auth = ack == .badUsernameOrPassword || ack == .notAuthorized
            teardown(announce: false)
            continuation.yield(.disconnected(reason: "broker refused the connection (\(ack))", authFailed: auth, requested: false))
        }
    }

    fileprivate func didDisconnect(_ link: Link, error: Error?) {
        guard link === self.link else { return }
        self.link = nil
        link.mqtt.delegate = nil
        for (_, waiter) in waiters { waiter.resume(returning: false) }
        waiters.removeAll()
        let reason = error.map { ($0 as NSError).localizedDescription } ?? "connection closed"
        continuation.yield(.disconnected(reason: reason, authFailed: false, requested: false))
    }

    fileprivate func didPublishAck(_ link: Link, id: UInt16) {
        guard link === self.link else { return }
        waiters.removeValue(forKey: id)?.resume(returning: true)
    }

    fileprivate func didSubscribe(_ link: Link, success: NSDictionary, failed: [String]) {
        guard link === self.link else { return }
        let ok = (success.allKeys as? [String]) ?? []
        continuation.yield(.subscribed(topics: ok, failed: failed))
    }

    fileprivate func didReceive(_ link: Link, message: CocoaMQTTMessage) {
        guard link === self.link else { return }
        continuation.yield(.message(topic: message.topic, payload: Data(message.payload), retained: message.retained))
    }

    /// One connection attempt's delegate; it forwards to the owner, which drops stale links.
    fileprivate final class Link: NSObject, CocoaMQTTDelegate {
        weak var owner: MQTTBrokerClient?
        let mqtt: CocoaMQTT
        let kind: BrokerEndpoint.Kind
        var connected = false

        init(owner: MQTTBrokerClient, mqtt: CocoaMQTT, kind: BrokerEndpoint.Kind) {
            self.owner = owner
            self.mqtt = mqtt
            self.kind = kind
        }

        func mqtt(_ mqtt: CocoaMQTT, didConnectAck ack: CocoaMQTTConnAck) { owner?.connectAck(self, ack) }
        func mqtt(_ mqtt: CocoaMQTT, didPublishMessage message: CocoaMQTTMessage, id: UInt16) {}
        func mqtt(_ mqtt: CocoaMQTT, didPublishAck id: UInt16) { owner?.didPublishAck(self, id: id) }
        func mqtt(_ mqtt: CocoaMQTT, didReceiveMessage message: CocoaMQTTMessage, id: UInt16) { owner?.didReceive(self, message: message) }
        func mqtt(_ mqtt: CocoaMQTT, didSubscribeTopics success: NSDictionary, failed: [String]) {
            owner?.didSubscribe(self, success: success, failed: failed)
        }
        func mqtt(_ mqtt: CocoaMQTT, didUnsubscribeTopics topics: [String]) {}
        func mqttDidPing(_ mqtt: CocoaMQTT) {}
        func mqttDidReceivePong(_ mqtt: CocoaMQTT) {}
        func mqttDidDisconnect(_ mqtt: CocoaMQTT, withError err: Error?) { owner?.didDisconnect(self, error: err) }
    }
}
