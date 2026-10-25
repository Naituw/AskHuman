import XCTest
@testable import AHProtocol
@testable import AHStore

final class PushDecryptorTests: XCTestCase {
    private let macId = "mac-home"
    private let key = Data(repeating: 7, count: 32)

    private func makeVault() throws -> Vault {
        let vault = Vault(secrets: MemorySecretStore())
        try vault.savePairing(PairingRecord(
            macId: macId, macName: "Home", phoneId: "p", kid: 1, m2p: key, p2m: Data(repeating: 9, count: 32),
            pairedAt: Date(timeIntervalSince1970: 0), caps: Caps(newTask: false, interject: false, watch: false)
        ))
        return vault
    }

    private func plain(kind: String = "ask") -> PushPlain {
        PushPlain(k: kind, rid: "req-1", mac: "Home", agent: "Claude Code", project: "relay", text: "Pick a database", ts: 1_700_000_000_000)
    }

    private func userInfo(sealedWith key: Data, scope: EnvelopeScope = .push, sender: String? = nil, plain: PushPlain) throws -> [AnyHashable: Any] {
        let body = try AHJSON.encoder.encode(plain)
        let envelope = AHEnvelope.seal(key: key, scope: scope, sender: sender ?? macId, kid: 1, plaintext: body)
        let object = try JSONSerialization.jsonObject(with: AHEnvelope.encode(envelope))
        return ["aps": ["alert": ["loc-key": "PUSH_ASK"]], "ah": object]
    }

    func testDecryptsAPushFromAPairedMac() throws {
        let vault = try makeVault()
        let result = PushDecryptor.decrypt(userInfo: try userInfo(sealedWith: key, plain: plain()), vault: vault)
        let push = try result.get()
        XCTAssertEqual(push.macId, macId)
        XCTAssertEqual(push.plain, plain())
        XCTAssertEqual(push.kind, .ask)
    }

    func testKindsAreMapped() {
        XCTAssertEqual(DecryptedPush.Kind("confirm"), .confirm)
        XCTAssertEqual(DecryptedPush.Kind("follow.done"), .followDone)
        XCTAssertEqual(DecryptedPush.Kind("follow.waiting"), .followWaiting)
        XCTAssertEqual(DecryptedPush.Kind("follow.failed"), .followFailed)
        XCTAssertEqual(DecryptedPush.Kind("resolved"), .resolved)
        XCTAssertEqual(DecryptedPush.Kind("from-the-future"), .other("from-the-future"))
    }

    func testAPlainSystemNotificationHasNothingToDecrypt() throws {
        let result = PushDecryptor.decrypt(userInfo: ["aps": ["alert": "hi"]], vault: try makeVault())
        XCTAssertEqual(result, .failure(.noPayload))
    }

    func testAMacThatIsNotPairedIsRefused() throws {
        let info = try userInfo(sealedWith: key, sender: "mac-stranger", plain: plain())
        XCTAssertEqual(PushDecryptor.decrypt(userInfo: info, vault: try makeVault()), .failure(.unknownMac))
    }

    func testAForeignKeyDoesNotAuthenticate() throws {
        let info = try userInfo(sealedWith: Data(repeating: 1, count: 32), plain: plain())
        XCTAssertEqual(PushDecryptor.decrypt(userInfo: info, vault: try makeVault()), .failure(.notAuthentic))
    }

    func testACiphertextFromAnotherDomainDoesNotAuthenticate() throws {
        // A message that travelled on a topic must not be replayable as a push.
        let info = try userInfo(sealedWith: key, scope: .topic("ah/u/d/p"), plain: plain())
        XCTAssertEqual(PushDecryptor.decrypt(userInfo: info, vault: try makeVault()), .failure(.notAuthentic))
    }

    func testGarbageIsMalformed() throws {
        let vault = try makeVault()
        XCTAssertEqual(PushDecryptor.decrypt(userInfo: ["ah": "not an object"], vault: vault), .failure(.malformed))
        XCTAssertEqual(PushDecryptor.decrypt(userInfo: ["ah": ["v": 1]], vault: vault), .failure(.malformed))
        XCTAssertEqual(PushDecryptor.decrypt(userInfo: ["ah": ["v": 9, "f": macId, "k": 1, "n": "AAAA", "c": "AAAA"]], vault: vault), .failure(.malformed))
    }

    func testPlaintextThatIsNotAPushIsMalformed() throws {
        let body = Data("{\"hello\":1}".utf8)
        let envelope = AHEnvelope.seal(key: key, scope: .push, sender: macId, kid: 1, plaintext: body)
        let object = try JSONSerialization.jsonObject(with: AHEnvelope.encode(envelope))
        XCTAssertEqual(PushDecryptor.decrypt(userInfo: ["ah": object], vault: try makeVault()), .failure(.malformed))
    }
}
