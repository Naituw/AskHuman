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
        XCTAssertThrowsError(try AHEnvelope.parse(Data(count: Wire.maxEnvelopeBytes + 1))) {
            XCTAssertEqual($0 as? EnvelopeError, .tooLarge)
        }
    }

    /// `z` marks a compressed plaintext (docs/specs/ios-traffic-reduction.md §3.3). Only the number 1 means something;
    /// anything else is refused, so that a later algorithm can never be mistaken for this one.
    func testOnlyTheNumberOneIsACompressionMarker() throws {
        let plain = try AHEnvelope.parse(Data(#"{"v":1,"f":"a","k":1,"n":"x","c":"y"}"#.utf8))
        XCTAssertNil(plain.z)
        let marked = try AHEnvelope.parse(Data(#"{"v":1,"f":"a","k":1,"n":"x","c":"y","z":1}"#.utf8))
        XCTAssertEqual(marked.z, 1)
        for value in ["2", "0", "-1", "true", "false", "1.0", "1.5", "\"1\"", "null", "[1]", "{}"] {
            let text = #"{"v":1,"f":"a","k":1,"n":"x","c":"y","z":"# + value + "}"
            XCTAssertThrowsError(try AHEnvelope.parse(Data(text.utf8)), "z = \(value)") {
                XCTAssertEqual($0 as? EnvelopeError, .malformed, "z = \(value)")
            }
        }
    }

    // MARK: compression

    func testCompressedEnvelopesFromTheFixtureOpenToTheirPlaintext() throws {
        let samples = try XCTUnwrap(fx["compressedEnvelopes"] as? [[String: Any]], "run UPDATE_WIRE_FIXTURES=1 cargo test ios::fixtures")
        XCTAssertFalse(samples.isEmpty)
        for item in samples {
            let name = item["name"] as! String
            let key = hex(item["key"])
            let topic = item["topic"] as! String
            let text = item["envelope"] as! String
            let env = try AHEnvelope.parse(Data(text.utf8))
            XCTAssertEqual(env.z, 1, name)

            // What Rust compressed and encrypted, the phone decrypts and inflates to the same bytes, UTF-8 included.
            let opened = try AHEnvelope.open(key: key, scope: .topic(topic), envelope: env)
            let plaintext = try XCTUnwrap(item["plaintext"] as? String)
            XCTAssertEqual(String(decoding: opened, as: UTF8.self), plaintext, name)
            XCTAssertTrue(plaintext.contains("\u{91cd}\u{6784}\u{6a21}\u{5757}"), "the sample carries a title that is not ASCII")
            XCTAssertLessThan(text.utf8.count * 2, plaintext.utf8.count, "it is worth compressing")
            let message = try Plain.open(key: key, scope: .topic(topic), envelope: env)
            XCTAssertEqual(message.t, "agents.snapshot", name)

            // The marker is part of what is authenticated, and the scope still binds.
            var unmarked = env
            unmarked.z = nil
            XCTAssertThrowsError(try AHEnvelope.open(key: key, scope: .topic(topic), envelope: unmarked), name)
            XCTAssertThrowsError(try AHEnvelope.open(key: key, scope: .topic(topic + "x"), envelope: env), name)
            // A push is never compressed: refused before anything is decrypted.
            XCTAssertThrowsError(try AHEnvelope.open(key: key, scope: .push, envelope: env), name) {
                XCTAssertEqual($0 as? EnvelopeError, .malformed)
            }
        }
    }

    func testTheAuthenticatedDataOfACompressedEnvelopeEndsInTheMarker() {
        XCTAssertEqual(String(decoding: AHEnvelope.aad(.topic("ah/u/d/p"), sender: "mac", kid: 1), as: UTF8.self), "ah1|ah/u/d/p|mac|1")
        XCTAssertEqual(
            String(decoding: AHEnvelope.aad(.topic("ah/u/d/p"), sender: "mac", kid: 1, compressed: true), as: UTF8.self), "ah1|ah/u/d/p|mac|1|z1"
        )
        XCTAssertEqual(String(decoding: AHEnvelope.aad(.push, sender: "mac", kid: 1), as: UTF8.self), "ah1|push|mac|1")
    }

    func testCompressionRoundTripsAndStopsAtTheLimit() throws {
        let text = Data(String(repeating: "a stable title of one agent, ", count: 100).utf8)
        let packed = AHEnvelope.compress(text)
        XCTAssertLessThan(packed.count, text.count / 4)
        XCTAssertEqual(try AHEnvelope.inflate(packed, limit: text.count), text, "exactly as long as allowed")
        XCTAssertThrowsError(try AHEnvelope.inflate(packed, limit: text.count - 1)) {
            XCTAssertEqual($0 as? EnvelopeError, .tooLarge, "one byte over")
        }

        // Bytes that do not compress at all still come back (stored blocks).
        let noise = Bytes.random(40_000)
        XCTAssertEqual(try AHEnvelope.inflate(AHEnvelope.compress(noise), limit: noise.count), noise)
    }

    func testADecompressionBombIsRefusedBeforeItFillsTheMemory() throws {
        let bomb = AHEnvelope.compress(Data(count: 16 * 1024 * 1024))
        XCTAssertLessThan(bomb.count, 32 * 1024, "sixteen MiB of zeros in a few KiB")
        XCTAssertThrowsError(try AHEnvelope.inflate(bomb, limit: Wire.maxPlainBytes)) {
            XCTAssertEqual($0 as? EnvelopeError, .tooLarge)
        }

        // The same through a whole envelope: it authenticates, and still is not opened.
        let key = Bytes.random(32)
        let nonce = Bytes.random(AHCrypto.nonceLength)
        let aad = Data("ah1|ah/u/d/p|mac|1|z1".utf8)
        let ciphertext = try AHCrypto.seal(key: key, nonce: nonce, aad: aad, plaintext: bomb)
        let envelope = Envelope(f: "mac", k: 1, n: Bytes.b64u(nonce), c: Bytes.b64u(ciphertext), z: 1)
        XCTAssertThrowsError(try AHEnvelope.open(key: key, scope: .topic("ah/u/d/p"), envelope: envelope)) {
            XCTAssertEqual($0 as? EnvelopeError, .tooLarge)
        }
    }

    func testADamagedOrTruncatedStreamIsRefused() throws {
        let text = Data(String(repeating: "a stable title of one agent, ", count: 200).utf8)
        let packed = AHEnvelope.compress(text)
        XCTAssertThrowsError(try AHEnvelope.inflate(packed.prefix(packed.count / 2), limit: 1 << 20), "cut in the middle")
        XCTAssertThrowsError(try AHEnvelope.inflate(packed.prefix(packed.count - 1), limit: 1 << 20), "the last byte missing")
        XCTAssertThrowsError(try AHEnvelope.inflate(Data(repeating: 0xFF, count: 64), limit: 1 << 20), "not a stream")
        XCTAssertThrowsError(try AHEnvelope.inflate(Data(), limit: 1 << 20), "nothing")
    }
}
