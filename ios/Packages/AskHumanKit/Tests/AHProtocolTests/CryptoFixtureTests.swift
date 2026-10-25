import XCTest
@testable import AHProtocol

final class CryptoFixtureTests: XCTestCase {
    private var fx: [String: Any] = [:]

    override func setUpWithError() throws {
        fx = try Fixtures.json("crypto.json")
    }

    private func hex(_ value: Any?) -> Data { Bytes.fromHex(value as! String)! }

    func testBase32AndBase64urlVectors() {
        for item in fx["base32"] as! [[String: String]] {
            XCTAssertEqual(Bytes.base32Lower(Bytes.fromHex(item["hex"]!)!), item["text"])
        }
        for item in fx["base64url"] as! [[String: String]] {
            let bytes = Bytes.fromHex(item["hex"]!)!
            XCTAssertEqual(Bytes.b64u(bytes), item["text"])
            XCTAssertEqual(Bytes.b64uDecode(item["text"]!), bytes)
        }
        XCTAssertNil(Bytes.b64uDecode("AA=="), "padding is not allowed")
        XCTAssertNil(Bytes.b64uDecode("A+/"), "standard alphabet is not allowed")
        XCTAssertNil(Bytes.b64uDecode("A"), "a single character is not a base64 group")
    }

    func testX25519MatchesTheFixtureOnBothSides() throws {
        let x = fx["x25519"] as! [String: String]
        let mac = try AHCrypto.KeyPair(secret: Bytes.fromHex(x["macPrivate"]!)!)
        let phone = try AHCrypto.KeyPair(secret: Bytes.fromHex(x["phonePrivate"]!)!)
        XCTAssertEqual(Bytes.hex(mac.publicKey), x["macPublic"])
        XCTAssertEqual(Bytes.hex(phone.publicKey), x["phonePublic"])
        XCTAssertEqual(Bytes.hex(mac.sharedSecret(with: phone.publicKey)!), x["shared"])
        XCTAssertEqual(Bytes.hex(phone.sharedSecret(with: mac.publicKey)!), x["shared"])
    }

    func testALowOrderPeerKeyIsRefused() throws {
        let key = AHCrypto.KeyPair()
        XCTAssertNil(key.sharedSecret(with: Data(repeating: 0, count: 32)))
    }

    func testTicketKey() {
        let t = fx["ticket"] as! [String: String]
        let key = AHCrypto.ticketKey(ticketSecret: Bytes.fromHex(t["ticketSecret"]!)!, ticketId: t["ticketId"]!)
        XCTAssertEqual(Bytes.hex(key), t["key"])
    }

    func testPairKeysAndVerificationCode() {
        let p = fx["pair"] as! [String: Any]
        let keys = p["keys"] as! [String: String]
        let derived = AHCrypto.derivePairKeys(
            sharedSecret: hex(p["shared"]),
            context: .init(
                macId: p["macId"] as! String,
                phoneId: p["phoneId"] as! String,
                ticketId: p["ticketId"] as! String,
                ticketSecret: hex(p["ticketSecret"]),
                macPublic: hex(p["macPublic"]),
                phonePublic: hex(p["phonePublic"])
            )
        )
        XCTAssertEqual(Bytes.hex(derived.m2p), keys["m2p"])
        XCTAssertEqual(Bytes.hex(derived.p2m), keys["p2m"])
        XCTAssertEqual(Bytes.hex(derived.sas), keys["sas"])
        XCTAssertEqual(AHCrypto.sasCode(kSas: derived.sas), p["sasCode"] as? String)
    }

    func testEnvelopesFromTheFixtureOpenAndReseal() throws {
        for item in fx["envelopes"] as! [[String: Any]] {
            let name = item["name"] as! String
            let key = hex(item["key"])
            let scope: EnvelopeScope = (item["scope"] as! String) == "push"
                ? .push : .topic(item["topic"] as! String)
            let env = try AHEnvelope.parse(Data((item["envelope"] as! String).utf8))
            XCTAssertEqual(env.f, item["from"] as? String, name)
            XCTAssertEqual(env.k, item["kid"] as? Int, name)

            let opened = try AHEnvelope.open(key: key, scope: scope, envelope: env)
            XCTAssertEqual(String(decoding: opened, as: UTF8.self), item["plaintext"] as? String, name)

            // Sealing the same plaintext with the same nonce must give the same envelope.
            let resealed = AHEnvelope.seal(
                key: key, scope: scope, sender: env.f, kid: env.k, nonce: hex(item["nonce"]), plaintext: opened
            )
            XCTAssertEqual(resealed, env, name)
        }
    }

    func testAnyTamperingOfTheEnvelopeIsRejected() throws {
        let item = (fx["envelopes"] as! [[String: Any]])[0]
        let key = hex(item["key"])
        let topic = item["topic"] as! String
        let env = try AHEnvelope.parse(Data((item["envelope"] as! String).utf8))

        XCTAssertThrowsError(try AHEnvelope.open(key: key, scope: .topic(topic + "x"), envelope: env))
        XCTAssertThrowsError(try AHEnvelope.open(key: key, scope: .push, envelope: env))
        var other = env
        other.f = "someone"
        XCTAssertThrowsError(try AHEnvelope.open(key: key, scope: .topic(topic), envelope: other))
        other = env
        other.k = 2
        XCTAssertThrowsError(try AHEnvelope.open(key: key, scope: .topic(topic), envelope: other))

        var ciphertext = Bytes.b64uDecode(env.c)!
        for index in 0..<ciphertext.count {
            ciphertext[index] ^= 0x01
            var tampered = env
            tampered.c = Bytes.b64u(ciphertext)
            XCTAssertThrowsError(try AHEnvelope.open(key: key, scope: .topic(topic), envelope: tampered), "byte \(index)")
            ciphertext[index] ^= 0x01
        }
    }

    func testEnvelopeParsingRules() throws {
        XCTAssertThrowsError(try AHEnvelope.parse(Data("{".utf8)))
        XCTAssertThrowsError(try AHEnvelope.parse(Data(#"{"v":2,"f":"a","k":1,"n":"x","c":"y"}"#.utf8))) {
            XCTAssertEqual($0 as? EnvelopeError, .version)
        }
        XCTAssertThrowsError(try AHEnvelope.parse(Data(#"{"v":1,"f":"a","k":1,"n":"x","c":"y","z":1}"#.utf8))) {
            XCTAssertEqual($0 as? EnvelopeError, .malformed)
        }
        XCTAssertThrowsError(try AHEnvelope.parse(Data(count: Wire.maxEnvelopeBytes + 1))) {
            XCTAssertEqual($0 as? EnvelopeError, .tooLarge)
        }
    }
}
