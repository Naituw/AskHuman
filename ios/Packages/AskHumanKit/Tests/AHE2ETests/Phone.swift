import Foundation
import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The real phone engine over a real MQTT client, with storage in a temp directory.
final class E2EPhone: @unchecked Sendable {
    let root: URL
    let secrets: MemorySecretStore
    let vault: Vault
    let store: LocalStore
    let engine: PhoneEngine
    private let lock = NSLock()
    private var collected: [EngineEvent] = []
    private var eventTask: Task<Void, Never>?

    init(
        deviceName: String = "E2E iPhone", bundleId: String = "com.example.e2e", secrets: MemorySecretStore = MemorySecretStore(),
        root: URL? = nil, broker: BrokerClient = MQTTBrokerClient()
    ) {
        self.root = root ?? FileManager.default.temporaryDirectory.appendingPathComponent("ah-e2e-phone-\(UUID().uuidString.prefix(8))")
        self.secrets = secrets
        vault = Vault(secrets: secrets)
        store = LocalStore(root: self.root.appendingPathComponent("store"))
        engine = PhoneEngine(
            vault: vault, store: store, broker: broker,
            config: EngineConfig(
                deviceName: deviceName, appVersion: "e2e", bundleId: bundleId, pushEnv: "dev", allowPlainLoopback: true,
                assetDirectory: self.root.appendingPathComponent("assets")
            )
        )
        eventTask = Task { [engine, weak self] in
            for await event in engine.events {
                self?.lock.withLock { self?.collected.append(event) }
            }
        }
    }

    deinit {
        eventTask?.cancel()
        try? FileManager.default.removeItem(at: root)
    }

    /// Test cases live until the whole run ends, so their phones must be cleaned up explicitly.
    func close() async {
        await engine.setForeground(false)
        eventTask?.cancel()
        try? FileManager.default.removeItem(at: root)
    }

    var events: [EngineEvent] { lock.withLock { collected } }
    func state() async -> EngineState { await engine.snapshot() }

    /// Waits until the phone shows `count` requests with their content, and returns them. A request
    /// may arrive as a summary first (the content follows in a second step, always so over a slow
    /// link), and an answer can only be given once the content is there.
    @discardableResult
    func requests(_ count: Int = 1, file: StaticString = #filePath, line: UInt = #line) async -> [InboxItem] {
        await eventually("\(count) request(s) with their content on the phone", file: file, line: line) {
            let inbox = await self.state().inbox
            return inbox.count == count && inbox.allSatisfy { $0.view != nil }
        }
        return await state().inbox
    }

    /// The only request on the phone, once its content is there.
    func request(file: StaticString = #filePath, line: UInt = #line) async throws -> InboxItem {
        try unwrap(await requests(1, file: file, line: line).first, file: file, line: line)
    }

    /// Pairs with the Mac behind `code`: compares the check codes, lets the Mac allow, waits for the first sync.
    func pair(with mac: MacInstance, code: String, file: StaticString = #filePath, line: UInt = #line) async throws {
        await engine.start()
        await engine.startPairing(code: code)
        await eventually("the Mac to ask for confirmation", file: file, line: line) {
            if let pairing = await self.state().pairing, pairing.stage == .macConfirming { return true }
            return false
        }
        let phoneSas = try unwrap(await state().pairing?.sas)
        var macSas: String?
        await eventually("the Mac to show the check code", file: file, line: line) {
            let pairing = (try? await mac.status())?["pairing"] as? [String: Any]
            macSas = pairing?["sas"] as? String
            return macSas != nil
        }
        XCTAssertEqual(phoneSas, macSas, "both sides must show the same check code", file: file, line: line)
        let allow = try await mac.cli(["channel", "ios", "allow"])
        XCTAssertEqual(allow.status, 0, allow.stderr, file: file, line: line)
        await eventually("pairing to finish", file: file, line: line) {
            if case .done = await self.state().pairing?.stage { return true }
            return false
        }
        await engine.finishPairing()
        await eventually("the first sync", file: file, line: line) { await self.state().macs.first?.link == .online }
    }
}

func unwrap<T>(_ value: T?, file: StaticString = #filePath, line: UInt = #line) throws -> T {
    try XCTUnwrap(value, file: file, line: line)
}

/// Polls until `check` holds.
func eventually(
    _ message: @autoclosure () -> String = "condition", timeout: TimeInterval = 20, file: StaticString = #filePath,
    line: UInt = #line, _ check: @escaping () async -> Bool
) async {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if await check() { return }
        try? await Task.sleep(nanoseconds: 50_000_000)
    }
    XCTFail("timed out waiting for: \(message())", file: file, line: line)
}
