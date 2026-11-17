import Compression
import Foundation

/// Envelope and plaintext message framing of wire protocol v1 (plan §3.2).
///
/// Envelope (UTF-8 JSON): `{"v":1,"f":"<sender id>","k":<kid>,"n":"<b64url 12B>","c":"<b64url ct‖tag>"}`.
/// Plaintext: `{"i":<msgId>,"t":<type>,"ts":<unix ms>,"b":<body>}`.
/// AEAD: ChaCha20-Poly1305 with AAD `"ah1|" + topic + "|" + f + "|" + k`; pushes use
/// `"ah1|push|" + f + "|" + k`.
///
/// Compression (docs/specs/ios-traffic-reduction.md §3.3): an envelope with `"z":1` carries the plaintext as raw
/// DEFLATE (RFC 1951, no zlib header or trailer), compressed before it was encrypted, and its AAD ends in `|z1`, so
/// the marker is authenticated. Only the Mac sends them, only on a topic and only to a phone that asked for them in
/// `hello`; a push is never compressed. The phone only opens them: what it sends is small.
public struct Envelope: Codable, Equatable, Sendable {
    public var v: Int
    public var f: String
    public var k: Int
    public var n: String
    public var c: String
    /// `1`: the plaintext was compressed (raw DEFLATE) before it was encrypted. `nil` otherwise.
    public var z: Int?

    public init(v: Int = Wire.proto, f: String, k: Int, n: String, c: String, z: Int? = nil) {
        self.v = v
        self.f = f
        self.k = k
        self.n = n
        self.c = c
        self.z = z
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
    static func aad(_ scope: EnvelopeScope, sender: String, kid: Int, compressed: Bool = false) -> Data {
        let base: String
        switch scope {
        case .topic(let topic): base = "ah1|\(topic)|\(sender)|\(kid)"
        case .push: base = "ah1|push|\(sender)|\(kid)"
        }
        // The marker is authenticated too: changing `z` makes the envelope fail to open.
        return Data((compressed ? base + "|z1" : base).utf8)
    }

    /// Parses and size/version-checks an envelope. A `z` field is accepted only as the number 1 (raw DEFLATE, see
    /// `open`); any other value, a boolean or a fraction included, is malformed, so a later algorithm can never be
    /// mistaken for this one.
    public static func parse(_ bytes: Data) throws -> Envelope {
        if bytes.count > Wire.maxEnvelopeBytes { throw EnvelopeError.tooLarge }
        guard let object = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any] else {
            throw EnvelopeError.malformed
        }
        if let marker = object["z"], !isCompressionMarker(marker) { throw EnvelopeError.malformed }
        guard let env = try? AHJSON.decoder.decode(Envelope.self, from: bytes) else {
            throw EnvelopeError.malformed
        }
        if env.v != Wire.proto { throw EnvelopeError.version }
        return env
    }

    /// Whether a JSON value is the integer 1 and nothing else that Foundation would convert to it (`true`, `1.0`).
    private static func isCompressionMarker(_ value: Any) -> Bool {
        guard let number = value as? NSNumber else { return false }
        switch String(cString: number.objCType) {
        case "c", "B", "d", "f": return false
        default: return number.intValue == 1
        }
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

    /// Decrypts an envelope and, when it carries `z`, inflates the result (at most `Wire.maxPlainBytes`). A push is
    /// never compressed, so `z` there is malformed.
    public static func open(key: Data, scope: EnvelopeScope, envelope env: Envelope) throws -> Data {
        let compressed: Bool
        switch env.z {
        case nil: compressed = false
        case .some(1) where scope != .push: compressed = true
        default: throw EnvelopeError.malformed
        }
        guard let nonce = Bytes.b64uDecode(env.n), nonce.count == AHCrypto.nonceLength,
              let ct = Bytes.b64uDecode(env.c)
        else { throw EnvelopeError.malformed }
        guard let plain = AHCrypto.open(
            key: key, nonce: nonce, aad: aad(scope, sender: env.f, kid: env.k, compressed: compressed), ciphertext: ct
        ) else { throw EnvelopeError.auth }
        return compressed ? try inflate(plain, limit: Wire.maxPlainBytes) : plain
    }

    /// Inflates raw DEFLATE (`COMPRESSION_ZLIB` of the Compression framework is the raw stream, without the zlib
    /// header), refusing a damaged or truncated stream and one that expands past `limit` bytes, so that a small
    /// message cannot be made to fill the memory. The output is never more than `limit` plus one chunk in memory.
    static func inflate(_ data: Data, limit: Int) throws -> Data {
        let chunk = 64 * 1024
        let scratch = UnsafeMutablePointer<UInt8>.allocate(capacity: chunk)
        defer { scratch.deallocate() }
        var stream = compression_stream(
            dst_ptr: scratch, dst_size: 0, src_ptr: UnsafePointer(scratch), src_size: 0, state: nil
        )
        guard compression_stream_init(&stream, COMPRESSION_STREAM_DECODE, COMPRESSION_ZLIB) == COMPRESSION_STATUS_OK else {
            throw EnvelopeError.malformed
        }
        defer { compression_stream_destroy(&stream) }

        var output = Data()
        output.reserveCapacity(min(limit, max(data.count * 4, 1_024)))
        return try data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) -> Data in
            // An empty input has no base address; the stream never reads it.
            stream.src_ptr = raw.bindMemory(to: UInt8.self).baseAddress ?? UnsafePointer(scratch)
            stream.src_size = raw.count
            while true {
                let sourceBefore = stream.src_size
                stream.dst_ptr = scratch
                stream.dst_size = chunk
                let status = compression_stream_process(&stream, Int32(COMPRESSION_STREAM_FINALIZE.rawValue))
                let produced = chunk - stream.dst_size
                output.append(scratch, count: produced)
                if output.count > limit { throw EnvelopeError.tooLarge }
                switch status {
                case COMPRESSION_STATUS_END:
                    return output
                case COMPRESSION_STATUS_OK:
                    // Neither input used nor output made: the stream stopped in the middle (it is truncated).
                    if produced == 0 && stream.src_size == sourceBefore { throw EnvelopeError.malformed }
                default:
                    throw EnvelopeError.malformed
                }
            }
        }
    }

    /// Compresses `data` as raw DEFLATE: what a `z: 1` envelope carries before it is encrypted. The phone does not
    /// send compressed messages; this is the Mac's side of the format, for tests that play the Mac.
    static func compress(_ data: Data) -> Data {
        guard !data.isEmpty else { return Data() }
        let capacity = data.count + data.count / 8 + 1_024
        let destination = UnsafeMutablePointer<UInt8>.allocate(capacity: capacity)
        defer { destination.deallocate() }
        let size = data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) -> Int in
            compression_encode_buffer(
                destination, capacity, raw.bindMemory(to: UInt8.self).baseAddress!, raw.count, nil, COMPRESSION_ZLIB
            )
        }
        precondition(size > 0, "compression failed")
        return Data(bytes: destination, count: size)
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
