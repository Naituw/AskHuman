import XCTest
@testable import AHProtocol
@testable import AHStore

final class VaultTests: XCTestCase {
    func testPhoneIdIsCreatedOnceAndStable() {
        let secrets = MemorySecretStore()
        let first = Vault(secrets: secrets).phoneId()
        XCTAssertTrue(Bytes.isValidId(first))
        XCTAssertEqual(Vault(secrets: secrets).phoneId(), first, "a new Vault over the same keychain sees the same id")
        XCTAssertNotEqual(Vault(secrets: MemorySecretStore()).phoneId(), first)
    }

    func testPairingsRoundTripAndOrder() throws {
        let vault = Vault(secrets: MemorySecretStore())
        XCTAssertTrue(vault.pairings().isEmpty)
        let caps = Caps(newTask: true, interject: false, watch: true)
        let a = PairingRecord(
            macId: "mac-a", macName: "Home", phoneId: "p", kid: 1, m2p: Data(repeating: 1, count: 32),
            p2m: Data(repeating: 2, count: 32), pairedAt: Date(timeIntervalSince1970: 200), caps: caps
        )
        var b = a
        b.macId = "mac-b"
        b.macName = "Office"
        b.pairedAt = Date(timeIntervalSince1970: 100)
        try vault.savePairing(a)
        try vault.savePairing(b)
        XCTAssertEqual(vault.pairings().map(\.macId), ["mac-b", "mac-a"], "oldest first")
        XCTAssertEqual(vault.pairing(macId: "mac-a"), a)
        XCTAssertNil(vault.pairing(macId: "nope"))

        b.alias = "Work Mac"
        try vault.savePairing(b)
        XCTAssertEqual(vault.pairing(macId: "mac-b")?.displayName, "Work Mac")
        XCTAssertEqual(vault.pairing(macId: "mac-a")?.displayName, "Home")

        vault.removePairing(macId: "mac-a")
        XCTAssertEqual(vault.pairings().map(\.macId), ["mac-b"])
    }

    func testBrokerAndPushSettings() {
        let vault = Vault(secrets: MemorySecretStore())
        XCTAssertNil(vault.broker)
        let broker = BrokerSettings(host: "b.example", port: 8883, wsPort: 8084, username: "u1", password: "pw", relayUrl: "https://r.example")
        vault.broker = broker
        XCTAssertEqual(vault.broker, broker)
        XCTAssertTrue(broker.sameAccount(as: BrokerSettings(host: "B.example", port: 1, wsPort: 2, username: "u1", password: "other", relayUrl: "")))
        XCTAssertFalse(broker.sameAccount(as: BrokerSettings(host: "b.example", port: 1, wsPort: 2, username: "u2", password: "pw", relayUrl: "")))
        XCTAssertFalse(broker.isLoopback)
        XCTAssertTrue(BrokerSettings(host: "127.0.0.1", port: 1883, wsPort: 0, username: "u", password: "", relayUrl: "").isLoopback)

        let reg = PushRegistration(handle: "h", env: "sandbox", expiresAt: 1_900_000_000, tokenDigest: "ab")
        vault.push = reg
        XCTAssertEqual(vault.push, reg)
        vault.push = nil
        XCTAssertNil(vault.push)
    }

    func testPairCodeBecomesBrokerSettings() {
        let code = PairCode(v: 1, h: "h", p: 8883, w: 8084, u: "u", pw: "p", r: "https://r", m: "m", mn: "n", t: "t", ts: "s", ek: "e", x: 1)
        let settings = BrokerSettings(code: code)
        XCTAssertEqual(settings, BrokerSettings(host: "h", port: 8883, wsPort: 8084, username: "u", password: "p", relayUrl: "https://r"))
    }
}

final class LocalStoreTests: XCTestCase {
    private var root: URL!
    private var store: LocalStore!

    override func setUpWithError() throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent("ah-store-\(UUID().uuidString)")
        store = LocalStore(root: root)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: root)
    }

    private func view(_ id: String) -> RequestView {
        RequestView(
            id: id, kind: "ask", createdAt: 1_700_000_000_000, agent: AgentRef(kind: "codex", label: "Codex"),
            project: ProjectRef(key: "k", name: "proj"), source: "cli", contentHash: "h",
            ask: AskView(
                isMarkdown: true, message: MessageView(text: "hi", files: []),
                questions: [QuestionView(text: "q", options: [OptionView(text: "a", recommended: false)])],
                selectOnly: false, single: false
            )
        )
    }

    func testRequestCacheRoundTrip() {
        let summary = RequestSummary(
            id: "r1", kind: "ask", project: ProjectRef(key: "k", name: "proj"), agent: AgentRef(kind: "codex", label: "Codex"),
            createdAt: 1, preview: "hi", contentHash: "h", size: 10
        )
        let item = CachedRequest(macId: "m", requestId: "r1", summary: summary, view: view("r1"), receivedAt: Date(timeIntervalSince1970: 5))
        store.saveRequest(item)
        XCTAssertEqual(store.cachedRequests(), [item])
        var read = item
        read.unread = false
        store.saveRequest(read)
        XCTAssertEqual(store.cachedRequests(), [read])
        store.removeRequest(macId: "m", requestId: "r1")
        XCTAssertTrue(store.cachedRequests().isEmpty)
    }

    func testEmptyDraftsAreNotKept() {
        let key = "m/r1"
        store.saveDraft(Draft(answers: [DraftAnswer(selected: [1], text: "x")]), key: key)
        XCTAssertEqual(store.draft(key: key)?.answers.first?.selected, [1])
        store.saveDraft(Draft(answers: [DraftAnswer()]), key: key)
        XCTAssertNil(store.draft(key: key), "an all-empty draft deletes the stored one")
    }

    func testOutboxKeepsOrder() {
        let body = JSONValue.object(["a": .int(1)])
        store.saveOutbox(OutboxItem(clientMsgId: "b", macId: "m", kind: .answer, subject: "r", type: "answer.submit", body: body, createdAt: Date(timeIntervalSince1970: 20)))
        store.saveOutbox(OutboxItem(clientMsgId: "a", macId: "m", kind: .confirm, subject: "r2", type: "confirm.decide", body: body, createdAt: Date(timeIntervalSince1970: 10)))
        XCTAssertEqual(store.outboxItems().map(\.clientMsgId), ["a", "b"])
        store.removeOutbox("a")
        XCTAssertEqual(store.outboxItems().map(\.clientMsgId), ["b"])
    }

    func testHistoryIsBoundedByCountAndAge() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        func entry(_ id: String, age: TimeInterval) -> HistoryEntry {
            HistoryEntry(
                id: id, macId: "m", macName: "Mac", request: view(id), outcome: .answered,
                receivedAt: now.addingTimeInterval(-age - 5), finishedAt: now.addingTimeInterval(-age)
            )
        }
        store.appendHistory(entry("old", age: LocalStore.historyMaxAge + 3600), now: now)
        XCTAssertTrue(store.history().isEmpty, "entries older than 90 days are pruned")

        for index in 0..<(LocalStore.historyLimit + 20) {
            store.appendHistory(entry("e\(index)", age: Double(index)), now: now)
        }
        let kept = store.history()
        XCTAssertEqual(kept.count, LocalStore.historyLimit)
        XCTAssertEqual(kept.first?.id, "e0", "newest first")
        XCTAssertFalse(kept.contains { $0.id == "e\(LocalStore.historyLimit + 5)" })

        store.appendHistory(entry("e0", age: 0), now: now)
        XCTAssertEqual(store.history().filter { $0.id == "e0" }.count, 1, "re-adding replaces")
    }

    func testForgetRemovesEverythingOfOneMac() {
        let now = Date()
        for mac in ["m1", "m2"] {
            store.appendHistory(HistoryEntry(id: "\(mac)-h", macId: mac, macName: mac, request: view("r"), outcome: .answered, receivedAt: now, finishedAt: now))
            store.saveMeta(MacMeta(lastEpoch: "e"), macId: mac)
            store.saveOutbox(OutboxItem(clientMsgId: "\(mac)-o", macId: mac, kind: .answer, subject: nil, type: "answer.submit", body: .null, createdAt: now))
        }
        store.forget(macId: "m1")
        XCTAssertEqual(store.history().map(\.macId), ["m2"])
        XCTAssertEqual(store.outboxItems().map(\.macId), ["m2"])
        XCTAssertNil(store.meta(macId: "m1").lastEpoch)
        XCTAssertEqual(store.meta(macId: "m2").lastEpoch, "e")
    }

    func testTrafficIsPerMonth() {
        XCTAssertEqual(store.traffic(month: "2026-10").sentBytes, 0)
        store.saveTraffic(TrafficStats(month: "2026-10", sentBytes: 5, receivedBytes: 7, sentMessages: 1, receivedMessages: 2))
        XCTAssertEqual(store.traffic(month: "2026-10").receivedBytes, 7)
        XCTAssertEqual(store.traffic(month: "2026-11").receivedBytes, 0, "a new month starts at zero")
    }

    func testTheFreeQuotaWarnsFromEightyPercentAndCountsEachMessageTwice() {
        let quota = TrafficStats.freeQuotaBytes
        func stats(_ bytes: UInt64) -> TrafficStats {
            TrafficStats(month: "2026-10", sentBytes: bytes / 2, receivedBytes: bytes - bytes / 2)
        }
        XCTAssertEqual(stats(1000).brokerEstimateBytes, 2000, "published once, delivered once")
        XCTAssertEqual(stats(quota / 4).quotaShare, 0.5, accuracy: 0.0001)
        XCTAssertEqual(stats(quota * 39 / 100).level, .ok)
        XCTAssertEqual(stats(quota * 40 / 100).level, .warning)
        XCTAssertEqual(stats(quota * 49 / 100).level, .warning)
        XCTAssertEqual(stats(quota / 2).level, .exhausted)
    }
}
