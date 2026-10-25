import Foundation
import CryptoKit
import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The Mac side of the protocol, just enough of it to exercise the phone engine: it listens on the
/// broker like the real gateway does, answers `hello`, serves requests, assets and uploads, and
/// records everything the phone sends so tests can assert on it.
final class TestMac: @unchecked Sendable {
    let user: String
    let macId: String
    var macName: String
    let clock: ManualClock
    let client: LoopbackClient
    var epoch = "epoch-\(UUID().uuidString.prefix(6))"

    let lock = NSLock()
    private var task: Task<Void, Never>?
    private var keys: [String: (kid: Int, m2p: Data, p2m: Data)] = [:]
    private var ticket: (id: String, secret: Data, key: Data, pair: AHCrypto.KeyPair)?
    private var knock: (phoneId: String, name: String, keys: AHCrypto.PairKeys, sas: String)?
    private var inbox: [RequestView] = []
    private var rev: UInt64 = 0
    private var assets: [String: Data] = [:]
    private var uploads: [String: (name: String, size: Int, chunks: Int, sha: String, data: Data, next: Int)] = [:]
    private var inboxLog: [(phoneId: String, plain: Plain)] = []
    private var answersSeen: [String: AnswerSubmit] = [:]

    /// Behaviors tests can switch.
    var autoHello = true
    /// A slow Mac: it answers `hello` with the ack only; `sendSnapshots(to:)` sends the rest.
    var holdSnapshots = false
    var answerStatus = "accepted"
    var ignoreAnswers = false
    var dropNextRequestGet = false
    var corruptAssetChecksum = false
    var caps = Caps(newTask: false, interject: false, watch: false)
    /// Number of upcoming `asset.get` requests to ignore.
    var dropAssetGets = 0
    /// Reason the Mac refuses uploads with (nil = accept).
    var uploadError: String?
    var ignoreUploads = false

    // Agents, follow, watch, interject and tasks: see `TestMac+Agents.swift`.
    var agents: [String: AgentView] = [:]
    var agentsRev: UInt64 = 1
    var followKeys: [String] = []
    var followRev: UInt64 = 1
    var slotRev: UInt64 = 1
    var slotActive = true
    /// What the Mac answers to `interject.send` (nil = no answer at all).
    var interjectReply: (status: String, reason: String?, pending: UInt64)? = ("queued", nil, 1)
    var interjectQueue: [String] = []
    var taskOptions = TaskOptions(
        enabled: true, permissionMode: "ask",
        workspaces: [WorkspaceOption(id: "ws1", label: "HumanInLoop", subtitle: "~/Developer", pinned: true, lastUsedAt: 0)],
        agents: [AgentOption(kind: "claude", label: "Claude Code", ready: true, subtitle: "Terminal", permissionSelectable: true)]
    )
    var todos: [TodoOption] = []
    /// The statuses the Mac sends, in order, after it received a `task.create`.
    var taskScript: [(stage: String, reason: String?, agentKey: String?)] = [("accepted", nil, nil), ("launched", nil, nil)]
    var ignoreTasks = false
    var ignoreTaskOptions = false
    var watchOpens: [String] = []
    var watchCloses: [String] = []

    init(broker: LoopbackBroker, user: String, clock: ManualClock, macId: String = Bytes.newId(), macName: String = "Home Mac") {
        self.user = user
        self.macId = macId
        self.macName = macName
        self.clock = clock
        client = broker.makeClient()
    }

    // MARK: lifecycle

    func start(password: String) async {
        let will = Presence(o: 0, t: clock.nowMs() / 1000, r: "mac")
        client.connect(
            endpoint: BrokerEndpoint(kind: .tcp, host: "broker.test", port: 1),
            params: BrokerConnectParams(
                clientId: "mac-\(macId)", username: user, password: password,
                will: BrokerWill(topic: Wire.topicPresence(user: user, deviceId: macId), payload: try! AHJSON.encoder.encode(will))
            )
        )
        task = Task { [weak self, client] in
            for await event in client.events {
                guard let self else { return }
                if case .message(let topic, let payload, _) = event { await self.on(topic: topic, payload: payload) }
            }
        }
        client.subscribe([Wire.topicUp(user: user, macId: macId), Wire.topicPresenceFilter(user: user)])
        await publishPresence(online: true)
    }

    func stop() {
        task?.cancel()
        client.disconnect()
    }

    func publishPresence(online: Bool) async {
        let presence = Presence(o: online ? 1 : 0, t: clock.nowMs() / 1000, r: "mac", e: online ? epoch : nil)
        _ = await client.publish(topic: Wire.topicPresence(user: user, deviceId: macId), payload: try! AHJSON.encoder.encode(presence), retain: true)
    }

    // MARK: pairing

    /// Starts a pairing and returns the code text the phone scans.
    func startPairing(relayUrl: String = "", password: String, expiresIn: TimeInterval = 300) -> String {
        let secret = Bytes.random(32)
        let id = Bytes.newId()
        let pair = AHCrypto.KeyPair()
        lock.lock()
        ticket = (id, secret, AHCrypto.ticketKey(ticketSecret: secret, ticketId: id), pair)
        lock.unlock()
        client.subscribe([Wire.topicPair(user: user, ticketId: id)])
        let code = PairCode(
            v: 1, h: "broker.test", p: 8883, w: 8084, u: user, pw: password, r: relayUrl, m: macId, mn: macName, t: id,
            ts: Bytes.b64u(secret), ek: Bytes.b64u(pair.publicKey), x: Int64(clock.now().timeIntervalSince1970 + expiresIn)
        )
        return code.encode()
    }

    /// The check code the Mac shows once the phone knocked.
    var sas: String? {
        lock.lock(); defer { lock.unlock() }
        return knock?.sas
    }

    var knockName: String? {
        lock.lock(); defer { lock.unlock() }
        return knock?.name
    }

    func allow() async {
        lock.lock()
        let pending = knock
        lock.unlock()
        guard let pending else { return }
        lock.lock()
        keys[pending.phoneId] = (Wire.kidPair, pending.keys.m2p, pending.keys.p2m)
        lock.unlock()
        await sendDown(PairAccept(macId: macId, macName: macName, caps: caps), to: pending.phoneId)
    }

    func reject(_ reason: String = "rejected") async {
        guard let t = lockedTicket() else { return }
        await sendOnTicket(PairReject(reason: reason), ticket: t)
    }

    private func lockedTicket() -> (id: String, secret: Data, key: Data, pair: AHCrypto.KeyPair)? {
        lock.lock(); defer { lock.unlock() }
        return ticket
    }

    private func sendOnTicket<M: WireMessage>(_ body: M, ticket t: (id: String, secret: Data, key: Data, pair: AHCrypto.KeyPair)) async {
        let topic = Wire.topicPair(user: user, ticketId: t.id)
        let plain = try! Plain(body, nowMs: clock.nowMs())
        let bytes = try! plain.sealed(key: t.key, scope: .topic(topic), sender: macId, kid: Wire.kidTicket)
        _ = await client.publish(topic: topic, payload: bytes, retain: false)
    }

    // MARK: sending

    @discardableResult
    func sendDown<M: WireMessage>(_ body: M, to phoneId: String) async -> Bool {
        lock.lock()
        let key = keys[phoneId]
        lock.unlock()
        guard let key else { return false }
        let topic = Wire.topicDown(user: user, phoneId: phoneId)
        let plain = try! Plain(body, nowMs: clock.nowMs())
        let bytes = try! plain.sealed(key: key.m2p, scope: .topic(topic), sender: macId, kid: key.kid)
        return await client.publish(topic: topic, payload: bytes, retain: false)
    }

    /// Sends a message built by hand (stale timestamps, wrong keys …).
    func sendRaw(plain: Plain, to phoneId: String, key: Data? = nil, kid: Int = Wire.kidPair) async {
        lock.lock()
        let stored = keys[phoneId]
        lock.unlock()
        let topic = Wire.topicDown(user: user, phoneId: phoneId)
        let bytes = try! plain.sealed(key: key ?? stored!.m2p, scope: .topic(topic), sender: macId, kid: kid)
        _ = await client.publish(topic: topic, payload: bytes, retain: false)
    }

    var pairedPhones: [String] {
        lock.lock(); defer { lock.unlock() }
        return Array(keys.keys)
    }

    // MARK: requests

    func deliver(_ view: RequestView, full: Bool = true, to phoneId: String? = nil) async {
        lock.lock()
        inbox.append(view)
        rev += 1
        let current = rev
        lock.unlock()
        for phone in phoneId.map({ [$0] }) ?? pairedPhones {
            await sendDown(RequestNew(rev: current, request: view, full: full), to: phone)
        }
    }

    func resolve(_ id: String, how: String = "answeredElsewhere", by: String? = "Telegram") async {
        lock.lock()
        inbox.removeAll { $0.id == id }
        rev += 1
        let current = rev
        lock.unlock()
        for phone in pairedPhones {
            await sendDown(RequestResolved(rev: current, requestId: id, how: how, by: by), to: phone)
        }
    }

    /// Bumps the inbox revision without telling the phone (a lost delta).
    func skipRevision() {
        lock.lock()
        rev += 1
        lock.unlock()
    }

    func register(asset id: String, data: Data) {
        lock.lock()
        assets[id] = data
        lock.unlock()
    }

    func uploaded(_ id: String) -> (name: String, data: Data)? {
        lock.lock(); defer { lock.unlock() }
        return uploads[id].map { ($0.name, $0.data) }
    }

    // MARK: inspection

    func received(_ type: String) -> [Plain] {
        lock.lock(); defer { lock.unlock() }
        return inboxLog.filter { $0.plain.t == type }.map(\.plain)
    }

    func count(_ type: String) -> Int { received(type).count }

    /// Everything the Mac could read, of any type.
    var totalReceived: Int {
        lock.lock(); defer { lock.unlock() }
        return inboxLog.count
    }

    func answer(for requestId: String) -> AnswerSubmit? {
        lock.lock(); defer { lock.unlock() }
        return answersSeen[requestId]
    }

    // MARK: inbound

    private func on(topic: String, payload: Data) async {
        if topic == Wire.topicPair(user: user, ticketId: lockedTicket()?.id ?? "-") {
            await onTicket(topic: topic, payload: payload)
            return
        }
        guard topic == Wire.topicUp(user: user, macId: macId), let env = try? AHEnvelope.parse(payload) else { return }
        lock.lock()
        let key = keys[env.f]
        lock.unlock()
        guard let key, env.k == key.kid, let plain = try? Plain.open(key: key.p2m, scope: .topic(topic), envelope: env) else { return }
        lock.lock()
        inboxLog.append((env.f, plain))
        lock.unlock()
        await handle(plain, from: env.f)
    }

    private func onTicket(topic: String, payload: Data) async {
        guard let t = lockedTicket(), let env = try? AHEnvelope.parse(payload), env.k == Wire.kidTicket,
              let plain = try? Plain.open(key: t.key, scope: .topic(topic), envelope: env), plain.t == Wire.T.pairRequest,
              let request = try? plain.body(as: PairRequest.self), let phonePublic = Bytes.b64uDecode(request.ek),
              let shared = t.pair.sharedSecret(with: phonePublic)
        else { return }
        let derived = AHCrypto.derivePairKeys(
            sharedSecret: shared,
            context: AHCrypto.PairContext(
                macId: macId, phoneId: request.phoneId, ticketId: t.id, ticketSecret: t.secret, macPublic: t.pair.publicKey,
                phonePublic: phonePublic
            )
        )
        lock.lock()
        knock = (request.phoneId, request.name, derived, AHCrypto.sasCode(kSas: derived.sas))
        lock.unlock()
        await sendOnTicket(PairPending(macId: macId), ticket: t)
    }

    private func handle(_ plain: Plain, from phoneId: String) async {
        switch plain.t {
        case Wire.T.hello:
            guard autoHello, let hello = try? plain.body(as: Hello.self) else { return }
            await sendHelloSync(to: phoneId, nonce: hello.nonce)
        case Wire.T.resync:
            if let resync = try? plain.body(as: Resync.self) {
                switch resync.stream {
                case "inbox": await sendInboxSnapshot(to: phoneId)
                case "agents": await sendAgentsSnapshot(to: phoneId)
                case "follow": await sendFollowState(to: phoneId)
                case "slot": await sendDown(SlotState(rev: slotRev, active: slotActive, autoActivation: false), to: phoneId)
                default: break
                }
            }
        case RequestGet.type:
            if dropNextRequestGet { dropNextRequestGet = false; return }
            if let get = try? plain.body(as: RequestGet.self) {
                lock.lock()
                let view = inbox.first { $0.id == get.requestId }
                lock.unlock()
                if let view { await sendDown(RequestFull(request: view), to: phoneId) }
            }
        case AnswerSubmit.type:
            guard !ignoreAnswers, let submit = try? plain.body(as: AnswerSubmit.self) else { return }
            lock.lock()
            answersSeen[submit.requestId] = submit
            lock.unlock()
            await sendDown(AnswerAck(clientMsgId: submit.clientMsgId, requestId: submit.requestId, status: answerStatus, reason: answerStatus == "invalid" ? "bad answer" : nil), to: phoneId)
            if answerStatus == "accepted" { await resolve(submit.requestId, how: "answered", by: nil) }
        case ConfirmDecide.type:
            guard !ignoreAnswers, let decide = try? plain.body(as: ConfirmDecide.self) else { return }
            await sendDown(ConfirmAck(clientMsgId: decide.clientMsgId, requestId: decide.requestId, status: answerStatus, reason: nil), to: phoneId)
            if answerStatus == "accepted" { await resolve(decide.requestId, how: "answered", by: nil) }
        case AssetGet.type:
            if dropAssetGets > 0 { dropAssetGets -= 1; return }
            if let get = try? plain.body(as: AssetGet.self) { await serveAsset(get, to: phoneId) }
        case UploadBegin.type:
            if ignoreUploads { return }
            if let begin = try? plain.body(as: UploadBegin.self) {
                if let uploadError {
                    await sendDown(UploadAck(uploadId: begin.uploadId, nextIndex: 0, status: "error", reason: uploadError), to: phoneId)
                    return
                }
                lock.lock()
                uploads[begin.uploadId] = (begin.name, Int(begin.size), Int(begin.chunks), begin.sha256, Data(), 0)
                lock.unlock()
                await sendDown(UploadAck(uploadId: begin.uploadId, nextIndex: 0, status: "ok", reason: nil), to: phoneId)
            }
        case UploadChunk.type:
            if let chunk = try? plain.body(as: UploadChunk.self) { await takeUploadChunk(chunk, from: phoneId) }
        default:
            await handleAgents(plain, from: phoneId)
        }
    }

    func sendHelloSync(to phoneId: String, nonce: String) async {
        await sendDown(
            HelloAck(nonce: nonce, epoch: epoch, macName: macName, caps: caps, slot: SlotInfo(active: slotActive, autoActivation: false)),
            to: phoneId
        )
        if holdSnapshots { return }
        await sendSnapshots(to: phoneId)
    }

    /// What follows the ack of a handshake (the phone is `synced` once the inbox snapshot is in).
    func sendSnapshots(to phoneId: String) async {
        await sendInboxSnapshot(to: phoneId)
        await sendAgentsSnapshot(to: phoneId)
        await sendFollowState(to: phoneId)
        await sendDown(SlotState(rev: slotRev, active: slotActive, autoActivation: false), to: phoneId)
    }

    func sendInboxSnapshot(to phoneId: String) async {
        lock.lock()
        let items = inbox.map { view in
            RequestSummary(
                id: view.id, kind: view.kind, project: view.project, agent: view.agent, createdAt: view.createdAt,
                preview: PhoneEngine.preview(of: view), contentHash: view.contentHash, size: 100
            )
        }
        let current = rev
        lock.unlock()
        await sendDown(InboxSnapshot(epoch: epoch, rev: current, items: items), to: phoneId)
    }

    private func serveAsset(_ get: AssetGet, to phoneId: String) async {
        lock.lock()
        let data = assets[get.assetId]
        lock.unlock()
        guard let data else {
            await sendDown(AssetError(assetId: get.assetId, reason: "notFound"), to: phoneId)
            return
        }
        let size = Wire.assetChunkBytes
        let total = max(1, (data.count + size - 1) / size)
        var sha = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
        if corruptAssetChecksum { sha = String(repeating: "0", count: 64) }
        let end = min(Int(get.from) + Int(get.count), total)
        for index in Int(get.from)..<end {
            let slice = data.subdata(in: (index * size)..<min((index + 1) * size, data.count))
            await sendDown(
                AssetChunk(assetId: get.assetId, index: UInt64(index), total: UInt64(total), size: UInt64(data.count), sha256: index == 0 ? sha : nil, data: Bytes.b64u(slice)),
                to: phoneId
            )
        }
    }

    private func takeUploadChunk(_ chunk: UploadChunk, from phoneId: String) async {
        lock.lock()
        guard var upload = uploads[chunk.uploadId] else {
            lock.unlock()
            return
        }
        if Int(chunk.index) == upload.next, let bytes = Bytes.b64uDecode(chunk.data) {
            upload.data.append(bytes)
            upload.next += 1
            uploads[chunk.uploadId] = upload
        }
        let next = upload.next
        let complete = next == upload.chunks
        lock.unlock()
        if complete || next % 4 == 0 {
            await sendDown(UploadAck(uploadId: chunk.uploadId, nextIndex: UInt64(next), status: complete ? "complete" : "ok", reason: nil), to: phoneId)
        }
    }
}

// MARK: - helpers

func sampleAsk(id: String = Bytes.newId(), text: String = "Which database should we use?", options: [String] = ["PostgreSQL", "SQLite"]) -> RequestView {
    RequestView(
        id: id, kind: "ask", createdAt: 1_800_000_000_000, agent: AgentRef(kind: "codex", label: "Codex"),
        project: ProjectRef(key: "proj", name: "HumanInLoop"), source: "cli", contentHash: "h-\(id.prefix(6))",
        ask: AskView(
            isMarkdown: true, message: MessageView(text: text, files: []),
            questions: [QuestionView(text: "Pick one", options: options.map { OptionView(text: $0, recommended: false) })],
            selectOnly: false, single: true
        )
    )
}

func sampleConfirm(id: String = Bytes.newId()) -> RequestView {
    RequestView(
        id: id, kind: "confirm", createdAt: 1_800_000_000_000, agent: AgentRef(kind: "claude", label: "Claude Code"),
        project: ProjectRef(key: "proj", name: "HumanInLoop"), source: "hook", contentHash: "c-\(id.prefix(6))",
        confirm: ConfirmView(
            title: "Run rm -rf build?", context: [], detail: ConfirmDetailView(summary: "Deletes the build folder", bodyMd: "`rm -rf build`"),
            choices: [
                ConfirmChoiceView(wireIndex: 0, label: "Allow", description: "", role: "primary", recommended: true),
                ConfirmChoiceView(wireIndex: 1, label: "Deny", description: "", role: "danger", recommended: false),
            ],
            defaultIndex: 0, input: nil, dismissIndex: 1, submitLabel: "Send", expiresAt: 1_900_000_000_000
        )
    )
}

struct TestFailure: LocalizedError {
    var message: String
    var errorDescription: String? { message }
}

/// A relay that counts registrations and hands out numbered handles.
final class FakeRelay: RelayRegistering, @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    private var failure: Error?
    var expiresAt: Int64 = 1_900_000_000

    var calls: Int { lock.withLock { count } }

    func fail(with error: Error?) { lock.withLock { failure = error } }

    func register(relayUrl: String, deviceToken: Data, env: String, bundleId: String) async throws -> RelayHandle {
        let (number, error): (Int, Error?) = lock.withLock {
            count += 1
            return (count, failure)
        }
        if let error { throw error }
        return RelayHandle(handle: "v1.handle-\(number)", expiresAt: expiresAt)
    }
}

/// Polls until `check` holds (real time, for the cross-task plumbing; the engine's own timers use
/// the manual clock).
func eventually(
    _ message: @autoclosure () -> String = "condition", timeout: TimeInterval = 4, file: StaticString = #filePath,
    line: UInt = #line, _ check: @escaping () async -> Bool
) async {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if await check() { return }
        try? await Task.sleep(nanoseconds: 5_000_000)
    }
    XCTFail("timed out waiting for: \(message())", file: file, line: line)
}
