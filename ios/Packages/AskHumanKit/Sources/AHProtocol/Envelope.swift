import Foundation

/// Envelope and plaintext message framing of wire protocol v1 (plan §3.2).
///
/// Envelope (UTF-8 JSON): `{"v":1,"f":"<sender id>","k":<kid>,"n":"<b64url 12B>","c":"<b64url ct‖tag>"}`.
/// Plaintext: `{"i":<msgId>,"t":<type>,"ts":<unix ms>,"b":<body>}`.
/// AEAD: ChaCha20-Poly1305 with AAD `"ah1|" + topic + "|" + f + "|" + k`; pushes use
/// `"ah1|push|" + f + "|" + k`.
public struct Envelope: Codable, Equatable, Sendable {
    public var v: Int
    public var f: String
    public var k: Int
    public var n: String
    public var c: String

    public init(v: Int = Wire.proto, f: String, k: Int, n: String, c: String) {
        self.v = v
        self.f = f
        self.k = k
        self.n = n
        self.c = c
    }
}

public enum EnvelopeError: Error, Equatable {
    case tooLarge
    case malformed
    case version
    case auth
}

/// Where an envelope travels; part of the authenticated data so a ciphertext cannot be replayed on
/// another topic or in the push domain.
public enum EnvelopeScope: Equatable, Sendable {
    case topic(String)
    case push
}

public enum AHEnvelope {
    static func aad(_ scope: EnvelopeScope, sender: String, kid: Int) -> Data {
        switch scope {
        case .topic(let topic): return Data("ah1|\(topic)|\(sender)|\(kid)".utf8)
        case .push: return Data("ah1|push|\(sender)|\(kid)".utf8)
        }
    }

    /// Parses and size/version-checks an envelope. A `z` field (reserved for compression) is rejected.
    public static func parse(_ bytes: Data) throws -> Envelope {
        if bytes.count > Wire.maxEnvelopeBytes { throw EnvelopeError.tooLarge }
        guard let object = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any] else {
            throw EnvelopeError.malformed
        }
        if object["z"] != nil { throw EnvelopeError.malformed }
        guard let env = try? AHJSON.decoder.decode(Envelope.self, from: bytes) else {
            throw EnvelopeError.malformed
        }
        if env.v != Wire.proto { throw EnvelopeError.version }
        return env
    }

    public static func encode(_ env: Envelope) -> Data {
        try! AHJSON.encoder.encode(env)
    }

    /// Seals with an explicit nonce (fixtures and tests); production code uses `seal`.
    public static func seal(
        key: Data, scope: EnvelopeScope, sender: String, kid: Int, nonce: Data, plaintext: Data
    ) -> Envelope {
        let ct = try! AHCrypto.seal(
            key: key, nonce: nonce, aad: aad(scope, sender: sender, kid: kid), plaintext: plaintext
        )
        return Envelope(f: sender, k: kid, n: Bytes.b64u(nonce), c: Bytes.b64u(ct))
    }

    public static func seal(key: Data, scope: EnvelopeScope, sender: String, kid: Int, plaintext: Data) -> Envelope {
        seal(key: key, scope: scope, sender: sender, kid: kid, nonce: Bytes.random(AHCrypto.nonceLength), plaintext: plaintext)
    }

    public static func open(key: Data, scope: EnvelopeScope, envelope env: Envelope) throws -> Data {
        guard let nonce = Bytes.b64uDecode(env.n), nonce.count == AHCrypto.nonceLength,
              let ct = Bytes.b64uDecode(env.c)
        else { throw EnvelopeError.malformed }
        guard let plain = AHCrypto.open(
            key: key, nonce: nonce, aad: aad(scope, sender: env.f, kid: env.k), ciphertext: ct
        ) else { throw EnvelopeError.auth }
        return plain
    }
}

/// Decrypted application message.
public struct Plain: Codable, Equatable, Sendable {
    /// Message id (base64url of 16 random bytes); receivers de-duplicate on it.
    public var i: String
    /// Message type, e.g. `request.new`.
    public var t: String
    /// Sender clock, unix milliseconds (replay window).
    public var ts: Int64
    public var b: JSONValue

    public init(i: String = Bytes.newMsgId(), t: String, ts: Int64, b: JSONValue) {
        self.i = i
        self.t = t
        self.ts = ts
        self.b = b
    }

    /// A message of a typed body.
    public init<M: WireMessage>(_ body: M, nowMs: Int64) throws {
        self.init(t: M.type, ts: nowMs, b: try AHJSON.tree(body))
    }

    /// Decodes the body as `M`; fails when the type string or the shape does not match.
    public func body<M: WireMessage>(as type: M.Type) throws -> M {
        guard t == M.type else { throw EnvelopeError.malformed }
        return try AHJSON.value(M.self, from: b)
    }

    /// Builds the wire bytes for one message.
    public func sealed(key: Data, scope: EnvelopeScope, sender: String, kid: Int) throws -> Data {
        let plaintext = try AHJSON.encoder.encode(self)
        return AHEnvelope.encode(AHEnvelope.seal(key: key, scope: scope, sender: sender, kid: kid, plaintext: plaintext))
    }

    public static func open(key: Data, scope: EnvelopeScope, envelope: Envelope) throws -> Plain {
        let plaintext = try AHEnvelope.open(key: key, scope: scope, envelope: envelope)
        guard let plain = try? AHJSON.decoder.decode(Plain.self, from: plaintext) else {
            throw EnvelopeError.malformed
        }
        return plain
    }
}

/// A typed message body with its wire type string.
public protocol WireMessage: Codable, Equatable, Sendable {
    static var type: String { get }
}
