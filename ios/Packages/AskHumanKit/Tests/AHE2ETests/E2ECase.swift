import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// A throwaway Mac (its own daemon and config), a local broker and a phone engine, torn down after
/// every test.
class E2ECase: XCTestCase {
    var environment: E2EEnvironment!
    var broker: LocalBroker!
    var mac: MacInstance!
    var phone: E2EPhone!

    override func setUp() async throws {
        environment = try E2EEnvironment.current()
        broker = try await LocalBroker(environment: environment)
        mac = try await MacInstance(environment: environment, broker: broker)
        phone = E2EPhone()
    }

    override func tearDown() async throws {
        await phone?.close()
        await mac?.stop()
        broker?.stop()
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
