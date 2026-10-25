import Foundation
import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// A phone engine wired to a loopback broker and a `TestMac`, all on one manual clock.
final class Harness: @unchecked Sendable {
    let clock = ManualClock()
    let broker = LoopbackBroker(accounts: ["u1": "pw", "u2": "pw2"])
    let phone: LoopbackClient
    let secrets = MemorySecretStore()
    let vault: Vault
    let store: LocalStore
    let root: URL
    let engine: PhoneEngine
    let mac: TestMac
    let relay: FakeRelay
    private let lock = NSLock()
    private var collected: [EngineEvent] = []
    private var eventTask: Task<Void, Never>?

    init(relay: FakeRelay = FakeRelay()) {
        self.relay = relay
        root = FileManager.default.temporaryDirectory.appendingPathComponent("ah-core-\(UUID().uuidString)")
        phone = broker.makeClient()
        vault = Vault(secrets: secrets)
        store = LocalStore(root: root.appendingPathComponent("store"))
        engine = PhoneEngine(
            vault: vault, store: store, broker: phone, clock: clock, relay: relay,
            config: EngineConfig(
                deviceName: "Test iPhone", appVersion: "1.0", bundleId: "com.example.app", pushEnv: "dev",
                allowPlainLoopback: false, assetDirectory: root.appendingPathComponent("assets")
            )
        )
        mac = TestMac(broker: broker, user: "u1", clock: clock)
        eventTask = Task { [engine, weak self] in
            for await event in engine.events {
                self?.lock.lock()
                self?.collected.append(event)
                self?.lock.unlock()
            }
        }
    }

    deinit {
        eventTask?.cancel()
        mac.stop()
        try? FileManager.default.removeItem(at: root)
    }

    var events: [EngineEvent] {
        lock.lock(); defer { lock.unlock() }
        return collected
    }

    func state() async -> EngineState { await engine.snapshot() }

    /// Pairs the engine with the Mac and waits for the first sync.
    func pair(relayUrl: String = "") async {
        await mac.start(password: "pw")
        await engine.start()
        let code = mac.startPairing(relayUrl: relayUrl, password: "pw")
        await engine.startPairing(code: code)
        await eventually("the Mac to see the pairing request") { self.mac.sas != nil }
        await mac.allow()
        await eventually("pairing to finish") {
            if case .done = await self.state().pairing?.stage { return true }
            return false
        }
        await engine.finishPairing()
        await eventually("the first sync") { await self.state().macs.first?.link == .online }
    }

    /// The retained presence of a device, as the broker holds it.
    func presence(of deviceId: String) -> Presence? {
        guard let data = broker.retainedPayload(Wire.topicPresence(user: "u1", deviceId: deviceId)) else { return nil }
        return try? AHJSON.decoder.decode(Presence.self, from: data)
    }

    var phoneId: String { vault.phoneId() }

    /// Moves the manual clock forward in steps until `condition` holds. The engine registers its
    /// timers from tasks of its own, so a single big jump could pass them by.
    func advance(
        until condition: @escaping () async -> Bool, step: TimeInterval = 1, limit: TimeInterval = 240,
        _ message: String = "condition", file: StaticString = #filePath, line: UInt = #line
    ) async {
        var spent = 0.0
        while spent <= limit {
            if await condition() { return }
            await clock.advance(by: step)
            spent += step
            try? await Task.sleep(nanoseconds: 3_000_000)
        }
        XCTFail("timed out advancing the clock: \(message)", file: file, line: line)
    }
}

extension EngineState {
    var firstInbox: InboxItem? { inbox.first }
}

/// `XCTUnwrap` cannot take an `await` expression; this can.
func unwrap<T>(_ value: T?, file: StaticString = #filePath, line: UInt = #line) throws -> T {
    try XCTUnwrap(value, file: file, line: line)
}
