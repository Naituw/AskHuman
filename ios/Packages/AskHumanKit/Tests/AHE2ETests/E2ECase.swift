import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// A throwaway Mac (its own daemon and config), a broker (local, or the real services when
/// `AH_E2E_REAL_FROM` is set) and a phone engine, torn down after every test.
class E2ECase: XCTestCase {
    var environment: E2EEnvironment!
    var broker: E2EBroker!
    var mac: MacInstance!
    var phone: E2EPhone!

    override func setUp() async throws {
        environment = try E2EEnvironment.current()
        if environment.usesRealServices {
            broker = try await RealBroker(environment: environment)
        } else {
            broker = try await LocalBroker(environment: environment)
        }
        mac = try await MacInstance(environment: environment, broker: broker)
        phone = E2EPhone(bundleId: environment.phoneBundleId)
    }

    override func tearDown() async throws {
        await keepEvidenceIfFailed()
        await phone?.close()
        await mac?.stop()
        broker?.stop()
    }

    /// `AH_E2E_KEEP_LOGS=<folder>`: a failed test leaves the Mac's daemon log and a summary of what
    /// the phone saw there (the throwaway directories are deleted right after). Neither holds a secret.
    private func keepEvidenceIfFailed() async {
        guard let folder = ProcessInfo.processInfo.environment["AH_E2E_KEEP_LOGS"], !folder.isEmpty,
              (testRun?.totalFailureCount ?? 0) > 0, let mac, let phone else { return }
        let label = name.filter { $0.isLetter || $0.isNumber || $0 == "_" }
        let target = URL(fileURLWithPath: folder).appendingPathComponent("\(label)-\(Int(Date().timeIntervalSince1970))")
        try? FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
        for log in (FileManager.default.enumerator(at: mac.configDir, includingPropertiesForKeys: nil)?.compactMap { $0 as? URL } ?? [])
        where log.pathExtension == "log" {
            try? FileManager.default.copyItem(at: log, to: target.appendingPathComponent(log.lastPathComponent))
        }
        let state = await phone.state()
        var lines = ["macs: " + state.macs.map { "\($0.name) link=\($0.link) pending=\($0.pendingCount)" }.joined(separator: "; ")]
        lines += state.inbox.map { "inbox \($0.requestId) hasView=\($0.view != nil) state=\($0.state)" }
        lines += phone.events.map { "event \($0)" }
        lines.append(mac.report())
        // A pairing in progress shows its code in the status: leave that part out.
        if var status = try? await mac.status() {
            status["pairing"] = nil
            lines.append("mac status: \(status)")
        }
        try? lines.joined(separator: "\n").write(to: target.appendingPathComponent("phone.txt"), atomically: true, encoding: .utf8)
    }

    func assertFinishes(_ cmd: Cmd, file: StaticString = #filePath, line: UInt = #line) async {
        let status = await cmd.wait(timeout: 20)
        XCTAssertEqual(status, 0, cmd.stderr, file: file, line: line)
    }

    func pair() async throws {
        let code = try await mac.startPairing()
        try await phone.pair(with: mac, code: code)
    }
}
