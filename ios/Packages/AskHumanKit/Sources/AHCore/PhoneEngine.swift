import Foundation
import CryptoKit
import AHProtocol
import AHStore
import AHTransport

/// Static configuration of the engine.
public struct EngineConfig: Sendable {
    public var deviceName: String
    public var appVersion: String
    public var bundleId: String
    /// APNs environment of this build: `dev` (Xcode installs) or `prod` (TestFlight, App Store).
    public var pushEnv: String
    /// Debug builds may reach a loopback test broker over plain TCP.
    public var allowPlainLoopback: Bool
    /// Where downloaded attachments are kept.
    public var assetDirectory: URL

    public init(
        deviceName: String, appVersion: String, bundleId: String, pushEnv: String, allowPlainLoopback: Bool,
        assetDirectory: URL
    ) {
        self.deviceName = deviceName
        self.appVersion = appVersion
        self.bundleId = bundleId
        self.pushEnv = pushEnv
        self.allowPlainLoopback = allowPlainLoopback
        self.assetDirectory = assetDirectory
    }
}

/// The session engine: owns the broker connection and one session per paired Mac, applies what the
/// Macs send, and carries what the user does back to them. It is an actor, so all of its state is
/// touched one event at a time; the UI sees immutable `EngineState` snapshots.
public actor PhoneEngine: RemoteBackend {
    // MARK: dependencies

    let vault: Vault
    let store: LocalStore
    let broker: BrokerClient
    let clock: EngineClock
    let relay: RelayRegistering
    var config: EngineConfig

    // MARK: outputs

    public nonisolated let states: AsyncStream<EngineState>
    public nonisolated let events: AsyncStream<EngineEvent>
    let stateOut: AsyncStream<EngineState>.Continuation
    let eventOut: AsyncStream<EngineEvent>.Continuation

    // MARK: connection state

    var started = false
    var foreground = false
    var phase: ConnectionPhase = .idle
    var settings: BrokerSettings?
    var endpoints: [BrokerEndpoint] = []
    var endpointIndex = 0
    var backoff = ReconnectBackoff()
    var everConnected = false
    /// Subscriptions still waiting for their SUBACK on the current connection.
    var pendingSubscriptions: Set<String> = []
    var subscribed = false
    var presenceSettled = false
    var foregroundPublished = false
    var clockSkew = false
    /// The token of the link probe in flight until it comes back (`PhoneEngine+Repair.swift`).
    var probeToken: Data?
    /// When a notification last made the engine look into each Mac's session: a burst of
    /// notifications must not become a burst of repairs.
    var lastRepairAt: [String: Date] = [:]
    var timers: [String: Task<Void, Never>] = [:]
    var timerSerial = 0
    var bucket: TokenBucket
    var eventLoop: Task<Void, Never>?

    // MARK: domain state

    var macs: [String: MacRuntime] = [:]
    var requests: [String: RequestRuntime] = [:]
    var outbox: [String: OutboxItem] = [:]
    var historyRevision = 0
    var traffic: TrafficStats?
    var pairingFlow: PairingFlow?
    /// Latest presence of every Mac device seen on this connection (also those not paired yet).
    var presenceCache: [String: Presence] = [:]
    /// Outcome of the last pairing, kept until the UI dismisses it.
    var terminalPairing: PairingState?
    var deviceToken: Data?
    var pushStatus: PushStatus = .noToken
    var pushSent: [String: String] = [:]
    var downloads: [String: Download] = [:]
    var uploadMailboxes: [String: UploadMailbox] = [:]
    var uploadOwners: [String: String] = [:]

    // MARK: agents, watches, interjections and tasks

    /// Watches the user has open (`AgentItem.id`): the Mac forgets them with the connection, so
    /// they are asked for again after every handshake.
    var wantedWatches: Set<String> = []
    var watches: [String: WatchState] = [:]
    /// The Agent of the last `follow.set` per Mac, to name the one a `limit` refusal is about.
    var followRequests: [String: String] = [:]
    var interjectBoxes: [String: ReplyMailbox<InterjectAck>] = [:]
    var interjectStateBoxes: [String: ReplyMailbox<InterjectState>] = [:]
    /// Todo requests waiting for their answer (`PhoneEngine+Todos.swift`), and the Macs whose todo
    /// changes the screen wants to hear about.
    var todoBoxes: [String: ReplyMailbox<TodoReply>] = [:]
    var wantedTodoWatches: Set<String> = []
    var taskOptions: [String: TaskOptionsState] = [:]
    var taskTodos: [String: [TodoOption]] = [:]
    var tasks: [TaskProgress] = []
    /// What was sent for each task, so a failed one can be sent again unchanged.
    var taskBodies: [String: TaskCreate] = [:]

    public init(
        vault: Vault, store: LocalStore, broker: BrokerClient, clock: EngineClock = SystemEngineClock(),
        relay: RelayRegistering = HTTPRelayClient(), config: EngineConfig
    ) {
        self.vault = vault
        self.store = store
        self.broker = broker
        self.clock = clock
        self.relay = relay
        self.config = config
        bucket = TokenBucket(rate: Wire.publishRatePerSec, burst: Wire.publishBurst, now: clock.now().timeIntervalSince1970)
        var stateCont: AsyncStream<EngineState>.Continuation!
        states = AsyncStream(bufferingPolicy: .bufferingNewest(1)) { stateCont = $0 }
        stateOut = stateCont
        var eventCont: AsyncStream<EngineEvent>.Continuation!
        events = AsyncStream(bufferingPolicy: .unbounded) { eventCont = $0 }
        eventOut = eventCont
    }

    deinit {
        stateOut.finish()
        eventOut.finish()
    }

    // MARK: lifecycle

    public func start() async {
        guard !started else { return }
        started = true
        settings = vault.broker
        for record in vault.pairings() {
            macs[record.macId] = MacRuntime(record: record, lastSeenAt: store.meta(macId: record.macId).lastSeenAt)
        }
        for item in store.outboxItems() { outbox[item.clientMsgId] = item }
        for cached in store.cachedRequests() where macs[cached.macId] != nil {
            var state = RequestInternalState.pending
            if let entry = outbox.values.first(where: { $0.macId == cached.macId && $0.subject == cached.requestId }) {
                state = .sending(clientMsgId: entry.clientMsgId, since: entry.createdAt)
            }
            requests[cached.id] = RequestRuntime(cached: cached, state: state)
        }
        traffic = store.traffic(month: Self.month(clock.now()))
        eventLoop = Task { [weak self, broker] in
            for await event in broker.events {
                guard let self else { return }
                await self.handle(event)
            }
        }
        publish()
    }

    /// How long the app may stay inactive (on screen, but not receiving events) before it says goodbye.
    /// Control Center, the notification center, the app switcher and system alerts all do that, and the
    /// user is still looking at the app: they are not a departure, and the Mac should not see one.
    public static let inactiveGrace: TimeInterval = 10

    /// The app is on screen but not receiving events. The connection stays; a stay that outlasts
    /// `inactiveGrace` says goodbye as if the app had left. Leaving (`setForeground(false)`, which a
    /// locked phone or the home screen leads to within moments) says goodbye at once, and coming back
    /// (`setForeground(true)`) keeps the connection.
    public func setInactive() async {
        // After a departure there is nothing to cut short, and a timer started now would end the next
        // stay in the foreground.
        guard foreground else { return }
        // Already counting: the stay began with the first call.
        guard timers["inactive"] == nil else { return }
        after("inactive", Self.inactiveGrace) { engine in await engine.setForeground(false) }
    }

    public func setForeground(_ value: Bool) async {
        // Either way the grace period is over: the app is active again, or it leaves right now.
        cancelTimer("inactive")
        guard foreground != value else { return }
        foreground = value
        if value {
            startConnecting()
        } else {
            await goodbye()
            // The goodbye suspends this actor while the offline presence goes out. When the app came back
            // in the meantime (a system alert, the notification center …), its `startConnecting` found the
            // old link still up and did nothing, and the goodbye then dropped that link: the app would
            // sit there disconnected, in the foreground, until the next change of scene phase.
            if foreground { startConnecting() }
        }
        publish()
    }

    public func refresh() async {
        guard foreground else { return }
        switch phase {
        case .backoff, .refused:
            phase = .idle
            backoff.reset()
            startConnecting()
        case .idle:
            startConnecting()
        case .connecting, .connected:
            break
        }
        for (id, mac) in macs where mac.online {
            if mac.sync.isHandshaken {
                await requestResync(id, stream: "inbox")
            } else {
                // The handshake never finished: a snapshot would be ignored without an epoch, so
                // start it over (the user is looking at a list that cannot fill up otherwise).
                await restartHello(id)
            }
        }
        publish()
    }

    /// Says goodbye (offline presence) and drops the connection.
    func goodbye() async {
        cancelTimer("connect")
        cancelTimer("reconnect")
        let wasConnected: Bool
        if case .connected = phase { wasConnected = true } else { wasConnected = false }
        if wasConnected, let settings {
            let presence = Presence(o: 0, t: clock.nowMs() / 1000, r: "phone", fg: 0)
            if let payload = try? AHJSON.encoder.encode(presence) {
                _ = await broker.publish(topic: Wire.topicPresence(user: settings.username, deviceId: vault.phoneId()), payload: payload, retain: true)
            }
        }
        broker.disconnect()
        resetConnectionState()
        phase = .idle
    }

    func resetConnectionState() {
        subscribed = false
        presenceSettled = false
        foregroundPublished = false
        pendingSubscriptions = []
        presenceCache = [:]
        probeToken = nil
        cancelTimer("settle")
        cancelTimer("probe")
        for id in macs.keys {
            macs[id]?.resetSession()
            cancelTimer("hello.retry." + id)
        }
        failTransfers(with: .macOffline)
        failInterjects(for: nil, with: .macOffline)
        failTodos(for: nil, with: .macOffline)
        watches = [:]
        interruptTasks()
    }

    // MARK: timers

    /// Runs `body` after `seconds` unless a timer with the same key is scheduled in the meantime
    /// or `cancelTimer` is called.
    func after(_ key: String, _ seconds: TimeInterval, _ body: @escaping @Sendable (PhoneEngine) async -> Void) {
        timers[key]?.cancel()
        timerSerial += 1
        let serial = timerSerial
        let clock = clock
        timers[key] = Task { [weak self] in
            do { try await clock.sleep(seconds: seconds) } catch { return }
            guard let self else { return }
            guard await self.timerIsCurrent(key, serial) else { return }
            await body(self)
        }
        timerSerials[key] = serial
    }

    var timerSerials: [String: Int] = [:]

    func timerIsCurrent(_ key: String, _ serial: Int) -> Bool {
        guard timerSerials[key] == serial else { return false }
        timerSerials[key] = nil
        timers[key] = nil
        return true
    }

    func cancelTimer(_ key: String) {
        timers.removeValue(forKey: key)?.cancel()
        timerSerials[key] = nil
    }

    // MARK: connecting

    var activeSettings: BrokerSettings? { pairingFlow?.settings ?? settings }

    func wantsConnection() -> Bool {
        guard foreground, activeSettings != nil else { return false }
        return !macs.isEmpty || pairingFlow != nil
    }

    func startConnecting() {
        guard wantsConnection(), let active = activeSettings else { return }
        switch phase {
        case .connecting, .connected: return
        case .idle, .backoff, .refused: break
        }
        cancelTimer("reconnect")
        endpoints = EndpointPlan.endpoints(
            host: active.host, port: active.port, wsPort: active.wsPort, lastTransport: active.lastTransport,
            allowPlainLoopback: config.allowPlainLoopback
        )
        endpointIndex = 0
        attemptConnect()
    }

    func attemptConnect() {
        guard let active = activeSettings, endpointIndex < endpoints.count else { return }
        let endpoint = endpoints[endpointIndex]
        phase = .connecting(via: endpoint.kind.rawValue)
        let phoneId = vault.phoneId()
        let will = Presence(o: 0, t: clock.nowMs() / 1000, r: "phone", fg: 0)
        let params = BrokerConnectParams(
            clientId: "ah-" + phoneId, username: active.username, password: active.password,
            keepAliveSecs: Wire.keepaliveSecs, connectTimeout: 8,
            will: (try? AHJSON.encoder.encode(will)).map {
                BrokerWill(topic: Wire.topicPresence(user: active.username, deviceId: phoneId), payload: $0)
            }
        )
        broker.connect(endpoint: endpoint, params: params)
        after("connect", params.connectTimeout + 4) { engine in
            await engine.connectFailed(reason: "connection timed out")
        }
        publish()
    }

    func connectFailed(reason: String) {
        guard case .connecting = phase else { return }
        broker.disconnect()
        if endpointIndex + 1 < endpoints.count {
            endpointIndex += 1
            attemptConnect()
        } else {
            scheduleReconnect(reason: reason)
        }
    }

    func scheduleReconnect(reason: String) {
        guard wantsConnection() else {
            phase = .idle
            publish()
            return
        }
        let delay = backoff.next()
        phase = .backoff(retryAt: clock.now().addingTimeInterval(delay), reason: reason)
        after("reconnect", delay) { engine in
            await engine.reconnectTimerFired()
        }
        publish()
    }

    func reconnectTimerFired() {
        if case .backoff = phase { phase = .idle }
        startConnecting()
    }

    // MARK: broker events

    func handle(_ event: BrokerEvent) async {
        switch event {
        case .connected(let via):
            await onConnected(via: via)
        case .disconnected(let reason, let authFailed, let requested):
            onDisconnected(reason: reason, authFailed: authFailed, requested: requested)
        case .subscribed(let topics, let failed):
            await onSubscribeResult(topics: topics, failed: failed)
        case .message(let topic, let payload, let retained):
            await onMessage(topic: topic, payload: payload, retained: retained)
        }
        publish()
    }

    func onConnected(via: BrokerEndpoint.Kind) async {
        guard case .connecting = phase, let active = activeSettings else { return }
        cancelTimer("connect")
        phase = .connected(via: via.rawValue)
        backoff.reset()
        everConnected = true
        if via != .tcp, via.rawValue != active.lastTransport {
            rememberTransport(via.rawValue)
        }
        let phoneId = vault.phoneId()
        var topics = [
            Wire.topicDown(user: active.username, phoneId: phoneId),
            Wire.topicPresenceFilter(user: active.username),
        ]
        if let flow = pairingFlow { topics.append(flow.pairTopic) }
        pendingSubscriptions = Set(topics)
        broker.subscribe(topics)
    }

    func rememberTransport(_ kind: String) {
        if pairingFlow != nil {
            pairingFlow?.settings.lastTransport = kind
        } else if var current = settings {
            current.lastTransport = kind
            settings = current
            vault.broker = current
        }
    }

    func onDisconnected(reason: String, authFailed: Bool, requested: Bool) {
        if requested { return }
        let wasConnected: Bool
        if case .connected = phase { wasConnected = true } else { wasConnected = false }
        cancelTimer("connect")
        resetConnectionState()
        if authFailed {
            phase = .refused("The broker refused the username or password.")
            failPairing(.authFailed)
            return
        }
        if wasConnected {
            endpointIndex = 0
            scheduleReconnect(reason: reason)
        } else if case .connecting = phase {
            if endpointIndex + 1 < endpoints.count {
                endpointIndex += 1
                attemptConnect()
            } else {
                scheduleReconnect(reason: reason)
            }
        }
    }

    func onSubscribeResult(topics: [String], failed: [String]) async {
        guard case .connected = phase else { return }
        if !failed.isEmpty {
            phase = .refused("The broker refused the subscription (check the broker's access rules).")
            broker.disconnect()
            resetConnectionState()
            failPairing(.failed("The broker refused the subscription."))
            return
        }
        pendingSubscriptions.subtract(topics)
        if let flow = pairingFlow, topics.contains(flow.pairTopic), !flow.requestSent {
            await sendPairRequest()
        }
        if !subscribed, pendingSubscriptions.isEmpty {
            subscribed = true
            after("settle", 2) { engine in await engine.presenceSettledFired() }
            for id in macs.keys.sorted() { await evaluateHello(id) }
        }
    }

    func presenceSettledFired() async {
        presenceSettled = true
        await ensureForegroundPresence()
        publish()
    }

    // MARK: own presence

    /// Publishes `fg = 1` once per connection, when the first Mac is in sync (or the presence
    /// messages have settled without any Mac being online).
    func ensureForegroundPresence() async {
        guard subscribed, !foregroundPublished, let active = activeSettings else { return }
        let anySynced = macs.values.contains { $0.sync == .synced }
        guard anySynced || presenceSettled else { return }
        foregroundPublished = true
        let presence = Presence(o: 1, t: clock.nowMs() / 1000, r: "phone", fg: 1)
        guard let payload = try? AHJSON.encoder.encode(presence) else { return }
        _ = await broker.publish(
            topic: Wire.topicPresence(user: active.username, deviceId: vault.phoneId()), payload: payload, retain: true
        )
    }

    // MARK: inbound

    func onMessage(topic: String, payload: Data, retained: Bool) async {
        guard let active = activeSettings else { return }
        count(received: payload.count)
        if let device = Wire.presenceDevice(user: active.username, topic: topic) {
            await onPresence(device: device, payload: payload)
            return
        }
        if topic == Wire.topicDown(user: active.username, phoneId: vault.phoneId()) {
            // The link probe publishes to this very topic: when it comes back, the connection works.
            if let token = probeToken, payload == token {
                probeToken = nil
                cancelTimer("probe")
                return
            }
            await onDown(topic: topic, payload: payload)
            return
        }
        if let flow = pairingFlow, topic == flow.pairTopic {
            await onPairTopic(payload: payload)
        }
    }

    func onPresence(device: String, payload: Data) async {
        guard let presence = try? AHJSON.decoder.decode(Presence.self, from: payload), presence.r == "mac" else { return }
        presenceCache[device] = presence
        guard var mac = macs[device] else { return }
        let wasOnline = mac.online
        if presence.o != 1 {
            if wasOnline {
                failTransfers(for: device, with: .macOffline)
                failInterjects(for: device, with: .macOffline)
                failTodos(for: device, with: .macOffline)
                dropWatches(of: device)
                interruptTasks(of: device)
            }
            mac.resetSession()
            cancelTimer("hello.retry." + device)
        }
        mac.presence = presence
        if presence.o == 1 {
            mac.lastSeenAt = clock.now()
            store.saveMeta(MacMeta(lastSeenAt: mac.lastSeenAt, lastEpoch: mac.epoch, follow: store.meta(macId: device).follow), macId: device)
        }
        macs[device] = mac
        await evaluateHello(device)
    }

    /// Opens a message from a paired Mac. Returns `nil` for anything that must be dropped silently:
    /// unknown sender, wrong key id, failed authentication, stale clock, duplicate.
    func openFromMac(topic: String, payload: Data) -> (macId: String, plain: Plain)? {
        guard let env = try? AHEnvelope.parse(payload), var mac = macs[env.f], env.k == mac.record.kid else { return nil }
        guard let plain = try? Plain.open(key: mac.record.m2p, scope: .topic(topic), envelope: env) else { return nil }
        guard abs(plain.ts - clock.nowMs()) <= Wire.clockSkewMs else {
            clockSkew = true
            return nil
        }
        guard mac.dedup.insert(plain.i, now: clock.now()) else { return nil }
        mac.lastSeenAt = clock.now()
        macs[env.f] = mac
        return (env.f, plain)
    }

    func onDown(topic: String, payload: Data) async {
        // The message that completes a pairing arrives with keys that are not stored yet.
        if pairingFlow != nil, await onPairAcceptCandidate(topic: topic, payload: payload) { return }
        guard let (macId, plain) = openFromMac(topic: topic, payload: payload) else { return }
        await dispatch(macId: macId, plain: plain)
    }

    func dispatch(macId: String, plain: Plain) async {
        do {
            switch plain.t {
            case Wire.T.helloAck: try await onHelloAck(macId, plain.body(as: HelloAck.self))
            case Wire.T.error: onError(macId, try plain.body(as: ErrorBody.self))
            case Wire.T.pong: break
            case Wire.T.macInfo: onMacInfo(macId, try plain.body(as: MacInfo.self))
            case Wire.T.unpaired: await onUnpaired(macId)
            case Wire.T.pushState: onPushState(macId, try plain.body(as: PushState.self))
            case InboxSnapshot.type: await onInboxSnapshot(macId, try plain.body(as: InboxSnapshot.self))
            case RequestNew.type: await onRequestNew(macId, try plain.body(as: RequestNew.self))
            case RequestFull.type: await onRequestFull(macId, try plain.body(as: RequestFull.self))
            case RequestResolved.type: await onRequestResolved(macId, try plain.body(as: RequestResolved.self))
            case AnswerAck.type: await onAnswerAck(macId, try plain.body(as: AnswerAck.self))
            case ConfirmAck.type: await onConfirmAck(macId, try plain.body(as: ConfirmAck.self))
            case SlotState.type: onSlotState(macId, try plain.body(as: SlotState.self))
            case AgentsSnapshot.type: await onAgentsSnapshot(macId, try plain.body(as: AgentsSnapshot.self))
            case AgentsDelta.type: await onAgentsDelta(macId, try plain.body(as: AgentsDelta.self))
            case FollowState.type: onFollowState(macId, try plain.body(as: FollowState.self))
            case FollowEvent.type: onFollowEvent(macId, try plain.body(as: FollowEvent.self))
            case WatchFrame.type: await onWatchFrame(macId, try plain.body(as: WatchFrame.self))
            case WatchClosed.type: onWatchClosed(macId, try plain.body(as: WatchClosed.self))
            case InterjectAck.type: onInterjectAck(macId, try plain.body(as: InterjectAck.self))
            case InterjectState.type: onInterjectState(macId, try plain.body(as: InterjectState.self))
            case InterjectConsumed.type: onInterjectConsumed(macId, try plain.body(as: InterjectConsumed.self))
            case TaskOptions.type: onTaskOptions(macId, try plain.body(as: TaskOptions.self))
            case TaskTodos.type: onTaskTodos(macId, try plain.body(as: TaskTodos.self))
            case TaskStatus.type: onTaskStatus(macId, try plain.body(as: TaskStatus.self))
            case TodoProjects.type: onTodoProjects(macId, try plain.body(as: TodoProjects.self))
            case ProjectTodos.type: onTodoList(macId, try plain.body(as: ProjectTodos.self))
            case TodoAck.type: onTodoAck(macId, try plain.body(as: TodoAck.self))
            case TodoChanged.type: onTodoChanged(macId, try plain.body(as: TodoChanged.self))
            case AssetChunk.type: await onAssetChunk(macId, try plain.body(as: AssetChunk.self))
            case AssetError.type: onAssetError(macId, try plain.body(as: AssetError.self))
            case UploadAck.type: onUploadAck(macId, try plain.body(as: UploadAck.self))
            default: break
            }
        } catch {
            // A body that does not decode is dropped; the stream resyncs on the next snapshot.
        }
    }

    // MARK: outbound

    /// Seals and publishes one message to a Mac. `false` when it could not go out.
    @discardableResult
    func send<M: WireMessage>(_ body: M, to macId: String) async -> Bool {
        guard let plain = try? Plain(body, nowMs: clock.nowMs()) else { return false }
        return await send(plain, to: macId)
    }

    func send(_ plain: Plain, to macId: String) async -> Bool {
        guard let mac = macs[macId], let active = activeSettings, case .connected = phase else { return false }
        let topic = Wire.topicUp(user: active.username, macId: macId)
        guard let bytes = try? plain.sealed(key: mac.record.p2m, scope: .topic(topic), sender: mac.record.phoneId, kid: mac.record.kid),
              bytes.count <= Wire.maxEnvelopeBytes
        else { return false }
        let wait = bucket.take(now: clock.now().timeIntervalSince1970)
        if wait > 0 { try? await clock.sleep(seconds: wait) }
        count(sent: bytes.count)
        return await broker.publish(topic: topic, payload: bytes, retain: false)
    }

    // MARK: traffic

    static func month(_ date: Date) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC") ?? .current
        let parts = calendar.dateComponents([.year, .month], from: date)
        return String(format: "%04d-%02d", parts.year ?? 0, parts.month ?? 0)
    }

    func count(sent bytes: Int) {
        var stats = traffic ?? TrafficStats(month: Self.month(clock.now()))
        if stats.month != Self.month(clock.now()) { stats = TrafficStats(month: Self.month(clock.now())) }
        stats.sentBytes += UInt64(bytes)
        stats.sentMessages += 1
        traffic = stats
    }

    func count(received bytes: Int) {
        var stats = traffic ?? TrafficStats(month: Self.month(clock.now()))
        if stats.month != Self.month(clock.now()) { stats = TrafficStats(month: Self.month(clock.now())) }
        stats.receivedBytes += UInt64(bytes)
        stats.receivedMessages += 1
        traffic = stats
    }

    // MARK: state publication

    func link(of mac: MacRuntime) -> MacLink {
        switch phase {
        case .idle, .connecting: return .connecting
        case .backoff, .refused: return .brokerDown
        case .connected:
            if mac.sync == .synced { return .online }
            if mac.online { return .connecting }
            // An explicit offline presence is an answer; no presence at all is one only after a grace period.
            if mac.presence != nil { return .macOffline }
            return presenceSettled ? .macOffline : .connecting
        }
    }

    func snapshot() -> EngineState {
        let pending = Dictionary(grouping: requests.values, by: { $0.cached.macId }).mapValues(\.count)
        let list = macs.values.sorted { $0.record.pairedAt < $1.record.pairedAt }.map { mac in
            MacState(
                id: mac.record.macId, name: mac.record.displayName, macName: mac.record.macName, alias: mac.record.alias,
                link: link(of: mac), caps: mac.caps ?? mac.record.caps, slot: mac.slot, pairedAt: mac.record.pairedAt,
                lastSeenAt: mac.lastSeenAt, pendingCount: pending[mac.record.macId] ?? 0, pushStatus: mac.pushStatus
            )
        }
        let inbox = requests.values.compactMap { runtime -> InboxItem? in
            guard let mac = macs[runtime.cached.macId] else { return nil }
            let state: RequestState
            switch runtime.state {
            case .pending: state = .pending
            case .sending(let id, let since): state = .sending(clientMsgId: id, since: since)
            case .failed(let id, let reason): state = .failed(clientMsgId: id, reason: reason)
            }
            return InboxItem(
                macId: runtime.cached.macId, macName: mac.record.displayName, requestId: runtime.cached.requestId,
                summary: runtime.cached.summary, view: runtime.cached.view, receivedAt: runtime.cached.receivedAt,
                unread: runtime.cached.unread, state: state
            )
        }.sorted { $0.summary.createdAt > $1.summary.createdAt }
        return EngineState(
            phase: phase, foreground: foreground, macs: list, inbox: inbox, pairing: pairingState(), clockSkew: clockSkew,
            push: pushStatus, historyRevision: historyRevision, traffic: traffic, brokerHost: activeSettings?.host,
            agents: agentItems(), watches: watches, taskOptions: taskOptions, taskTodos: taskTodos, tasks: tasks
        )
    }

    func publish() {
        stateOut.yield(snapshot())
    }

    func emit(_ event: EngineEvent) {
        eventOut.yield(event)
    }
}

// MARK: - runtime types

enum SyncPhase: Equatable {
    case none
    case helloSent(nonce: String, attempt: Int)
    case acked
    case synced

    /// The Mac answered `hello`: from there on it takes what the phone sends, snapshots or not.
    var isHandshaken: Bool { self == .acked || self == .synced }
}

struct MacRuntime {
    var record: PairingRecord
    var presence: Presence?
    var sync: SyncPhase = .none
    /// The latest hellos that went unanswered so far, oldest first: an ack for any of them counts.
    var helloNonces: [String] = []
    /// Rounds of hellos that went unanswered since the last ack, to pace the slow retries.
    var helloRounds = 0
    var epoch: String?
    var caps: Caps?
    var slot: SlotInfo?
    var slotRev: UInt64?
    var inboxRev: UInt64?
    var agents: [String: AgentView] = [:]
    var agentsRev: UInt64?
    /// Agent keys the Mac says are followed.
    var follow: Set<String> = []
    var followRev: UInt64?
    var pushStatus: String?
    var dedup = DedupWindow()
    var lastSeenAt: Date?

    init(record: PairingRecord, lastSeenAt: Date? = nil) {
        self.record = record
        self.lastSeenAt = lastSeenAt
    }

    var online: Bool { presence?.o == 1 }

    /// Whether `nonce` answers a hello that is still waiting for its ack. Several count, not only the
    /// latest: the Mac answers every hello it receives, and when it is slow the answer to an earlier
    /// attempt is the first to arrive. Once the handshake is done, the rest are void.
    func awaitsHelloAck(nonce: String) -> Bool {
        switch sync {
        case .none, .helloSent: return helloNonces.contains(nonce)
        case .acked, .synced: return false
        }
    }

    /// The broker link or the Mac went away: everything learned in the session is void.
    mutating func resetSession() {
        presence = nil
        sync = .none
        helloNonces = []
        helloRounds = 0
        epoch = nil
        slot = nil
        slotRev = nil
        inboxRev = nil
        agents = [:]
        agentsRev = nil
        followRev = nil
    }
}

enum RequestInternalState: Equatable {
    case pending
    case sending(clientMsgId: String, since: Date)
    case failed(clientMsgId: String, reason: String)
}

struct RequestRuntime {
    var cached: CachedRequest
    var state: RequestInternalState
    /// `request.get` is in flight.
    var fetching = false
    var seenSent = false
}
