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
///
/// Binary envelope (docs/specs/ios-attachment-traffic.md §5.1): the same message without the JSON text and the two
/// base64 layers, `0xA1 | flags | kid (u16 BE) | sender (16 raw bytes) | nonce (12) | ciphertext ‖ tag`. The
/// authenticated data is the same text as for a JSON envelope (the sender is written out as its 26 base32 characters),
/// so the format changes nothing a ciphertext is bound to. It goes only on a topic and only to a peer that declared
/// `binary` in `hello` / `hello.ack`; pairing messages and pushes stay JSON, and both formats are always accepted.
///
/// Blob frame (§5.2): a message whose plaintext is `0x01 | header_len (u32 BE) | header | bytes`, where the header is
/// the JSON of a `Plain` without the bytes it carries (`asset.chunk`, `upload.chunk`). Blobs are never compressed.
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

/// The wire format a message is sealed in.
public enum WireFormat: Sendable {
    /// The JSON envelope, which every peer reads.
    case json
    /// The binary envelope, for a peer that said it opens it. A sender or `kid` its header cannot hold goes as JSON
    /// (`EnvelopeFrame.encodeBinary`), and so does anything in the push scope.
    case binary
}

/// An envelope after parsing, whichever wire format it came in: every field as it is, with nothing left to decode.
public struct EnvelopeFrame: Equatable, Sendable {
    /// The sender's id as the 26 characters it is written with in the authenticated data.
    public var sender: String
    public var kid: Int
    public var nonce: Data
    /// The ciphertext followed by its 16-byte authentication tag.
    public var ciphertext: Data
    /// The plaintext was compressed (raw DEFLATE) before it was encrypted.
    public var compressed: Bool
    /// It came in the binary format.
    public var binary: Bool

    public init(sender: String, kid: Int, nonce: Data, ciphertext: Data, compressed: Bool = false, binary: Bool = false) {
        self.sender = sender
        self.kid = kid
        self.nonce = nonce
        self.ciphertext = ciphertext
        self.compressed = compressed
        self.binary = binary
    }

    /// The fields of a parsed JSON envelope, decoded. A `z` other than 1, or a nonce or ciphertext that is not base64url
    /// of the right shape, is malformed.
    public init(_ envelope: Envelope) throws {
        let compressed: Bool
        switch envelope.z {
        case nil: compressed = false
        case .some(1): compressed = true
        default: throw EnvelopeError.malformed
        }
        guard let nonce = Bytes.b64uDecode(envelope.n), nonce.count == AHCrypto.nonceLength,
              let ciphertext = Bytes.b64uDecode(envelope.c)
        else { throw EnvelopeError.malformed }
        self.init(sender: envelope.f, kid: envelope.k, nonce: nonce, ciphertext: ciphertext, compressed: compressed)
    }

    /// The JSON envelope with these fields.
    public var envelope: Envelope {
        Envelope(f: sender, k: kid, n: Bytes.b64u(nonce), c: Bytes.b64u(ciphertext), z: compressed ? 1 : nil)
    }

    /// The binary envelope with these fields, or `nil` when the sender or the `kid` does not fit its header: the sender
    /// must be an id in the exact form `Bytes.newId` gives out (it is carried as 16 raw bytes) and the `kid` must fit 16
    /// bits. A message that cannot go in it is sent in the JSON envelope, which any peer reads.
    public func encodeBinary() -> Data? {
        guard let senderBytes = Bytes.idToBytes(sender), kid >= 0, kid <= Int(UInt16.max),
              nonce.count == AHCrypto.nonceLength
        else { return nil }
        var out = Data(capacity: AHEnvelope.binaryHeaderLength + ciphertext.count)
        out.append(AHEnvelope.binaryMagic)
        out.append(compressed ? AHEnvelope.binaryCompressedFlag : 0)
        out.append(UInt8(kid >> 8))
        out.append(UInt8(kid & 0xff))
        out.append(senderBytes)
        out.append(nonce)
        out.append(ciphertext)
        return out
    }
}

/// What an envelope of either format holds once it is opened: a message, or a blob frame (a message header and the
/// raw bytes it carries).
public enum OpenedMessage: Equatable, Sendable {
    case message(Plain)
    case blob(head: Plain, bytes: Data)
}

public enum AHEnvelope {
    /// First byte of a binary envelope. A JSON envelope starts with `{`, so the two cannot be mistaken for each other.
    public static let binaryMagic: UInt8 = 0xA1
    /// First byte of the plaintext of a blob frame. A JSON message starts with `{`, so the two cannot be mistaken for
    /// each other.
    public static let blobMarker: UInt8 = 0x01
    /// Bit 0 of the flags byte of a binary envelope: the plaintext was compressed before it was encrypted. The other
    /// bits are reserved and must be zero.
    static let binaryCompressedFlag: UInt8 = 0b0000_0001
    /// Magic, flags, `kid` (u16), the sender id (16 raw bytes) and the nonce.
    static let binaryHeaderLength = 1 + 1 + 2 + 16 + AHCrypto.nonceLength
    /// Length of the ChaCha20-Poly1305 authentication tag at the end of every ciphertext.
    static let tagLength = 16
    /// The shortest binary envelope: the header and the tag of an empty plaintext.
    static let binaryMinLength = binaryHeaderLength + tagLength

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

    /// An envelope in the wire format of the caller's choice. The binary one is used only on a topic and only when the
    /// sender and `kid` fit its header (`EnvelopeFrame.encodeBinary`); anything else goes as JSON.
    public static func encode(_ frame: EnvelopeFrame, scope: EnvelopeScope, format: WireFormat) -> Data {
        if format == .binary, scope != .push, let bytes = frame.encodeBinary() { return bytes }
        return encode(frame.envelope)
    }

    /// Parses and size-checks an envelope of either wire format. A binary one is recognised by its first byte;
    /// everything else is read as JSON (`parse`).
    public static func parseFrame(_ bytes: Data) throws -> EnvelopeFrame {
        if bytes.first == binaryMagic { return try parseBinary(bytes) }
        return try EnvelopeFrame(parse(bytes))
    }

    private static func parseBinary(_ received: Data) throws -> EnvelopeFrame {
        if received.count > Wire.maxEnvelopeBytes { throw EnvelopeError.tooLarge }
        if received.count < binaryMinLength { throw EnvelopeError.malformed }
        let bytes = rebased(received)
        let flags = bytes[1]
        // A reserved bit: a later format must not be mistaken for this one.
        if flags & ~binaryCompressedFlag != 0 { throw EnvelopeError.malformed }
        return EnvelopeFrame(
            sender: Bytes.base32Lower(bytes.subdata(in: 4..<20)),
            kid: Int(bytes[2]) << 8 | Int(bytes[3]),
            nonce: bytes.subdata(in: 20..<binaryHeaderLength),
            ciphertext: bytes.subdata(in: binaryHeaderLength..<bytes.count),
            compressed: flags & binaryCompressedFlag != 0,
            binary: true
        )
    }

    /// The same bytes indexed from zero, whatever slice of a larger buffer they were.
    private static func rebased(_ data: Data) -> Data {
        data.startIndex == 0 ? data : Data(data)
    }

    /// Seals with an explicit nonce (fixtures and tests); production code uses `seal`.
    public static func seal(
        key: Data, scope: EnvelopeScope, sender: String, kid: Int, nonce: Data, plaintext: Data
    ) -> Envelope {
        sealFrame(key: key, scope: scope, sender: sender, kid: kid, nonce: nonce, plaintext: plaintext).envelope
    }

    public static func seal(key: Data, scope: EnvelopeScope, sender: String, kid: Int, plaintext: Data) -> Envelope {
        seal(key: key, scope: scope, sender: sender, kid: kid, nonce: Bytes.random(AHCrypto.nonceLength), plaintext: plaintext)
    }

    /// Encrypts `plaintext` as it is (`compressed` only says whether it already is raw DEFLATE, and is authenticated
    /// with it) into the fields of an envelope, whichever format they are written in afterwards.
    static func sealFrame(
        key: Data, scope: EnvelopeScope, sender: String, kid: Int, nonce: Data, plaintext: Data, compressed: Bool = false
    ) -> EnvelopeFrame {
        let ct = try! AHCrypto.seal(
            key: key, nonce: nonce, aad: aad(scope, sender: sender, kid: kid, compressed: compressed), plaintext: plaintext
        )
        return EnvelopeFrame(sender: sender, kid: kid, nonce: nonce, ciphertext: ct, compressed: compressed)
    }

    /// Decrypts an envelope and, when it carries `z`, inflates the result (at most `Wire.maxPlainBytes`). A push is
    /// never compressed, so `z` there is malformed.
    public static func open(key: Data, scope: EnvelopeScope, envelope env: Envelope) throws -> Data {
        try openFrame(key: key, scope: scope, frame: EnvelopeFrame(env))
    }

    /// `open` for an envelope of either wire format.
    public static func openFrame(key: Data, scope: EnvelopeScope, frame: EnvelopeFrame) throws -> Data {
        if frame.compressed && scope == .push { throw EnvelopeError.malformed }
        guard let plain = AHCrypto.open(
            key: key, nonce: frame.nonce,
            aad: aad(scope, sender: frame.sender, kid: frame.kid, compressed: frame.compressed),
            ciphertext: frame.ciphertext
        ) else { throw EnvelopeError.auth }
        return frame.compressed ? try inflate(plain, limit: Wire.maxPlainBytes) : plain
    }

    /// Opens an envelope of either format and reads the message in it. The first byte of the plaintext tells a blob
    /// frame (`0x01`) from a JSON message (`{`). A blob is never compressed, so a compressed envelope that inflates to
    /// one is refused, and a push never carries one.
    public static func openMessage(key: Data, scope: EnvelopeScope, frame: EnvelopeFrame) throws -> OpenedMessage {
        let plaintext = try openFrame(key: key, scope: scope, frame: frame)
        guard plaintext.first == blobMarker else {
            guard let plain = try? AHJSON.decoder.decode(Plain.self, from: plaintext) else {
                throw EnvelopeError.malformed
            }
            return .message(plain)
        }
        if frame.compressed || scope == .push { throw EnvelopeError.malformed }
        let (head, bytes) = try decodeBlob(plaintext)
        return .blob(head: head, bytes: bytes)
    }

    /// The plaintext of a blob frame: the marker, the length of the header (u32, big endian), the header, then the
    /// bytes. `head` is a `Plain` whose body leaves out the bytes (`data`). Fails when the header is longer than
    /// `Wire.blobHeaderMaxBytes` or the bytes are more than one chunk (`Wire.assetChunkBytes`).
    public static func encodeBlob(head: Plain, bytes: Data) throws -> Data {
        let header = try AHJSON.encoder.encode(head)
        guard header.count <= Wire.blobHeaderMaxBytes, bytes.count <= Wire.assetChunkBytes else {
            throw EnvelopeError.tooLarge
        }
        var out = Data(capacity: 1 + 4 + header.count + bytes.count)
        out.append(blobMarker)
        withUnsafeBytes(of: UInt32(header.count).bigEndian) { out.append(contentsOf: $0) }
        out.append(header)
        out.append(bytes)
        return out
    }

    /// The inverse of `encodeBlob`: the header message and the bytes after it. A plaintext over `Wire.maxPlainBytes`, a
    /// length that is over `Wire.blobHeaderMaxBytes` or past the end of the plaintext, a header that is not a message,
    /// or more bytes than one chunk (`Wire.assetChunkBytes`) is malformed.
    public static func decodeBlob(_ received: Data) throws -> (head: Plain, bytes: Data) {
        guard received.first == blobMarker, received.count <= Wire.maxPlainBytes, received.count >= 5 else {
            throw EnvelopeError.malformed
        }
        let plaintext = rebased(received)
        let headerLength = (0..<4).reduce(0) { $0 << 8 | Int(plaintext[1 + $1]) }
        guard headerLength <= Wire.blobHeaderMaxBytes, plaintext.count >= 5 + headerLength else {
            throw EnvelopeError.malformed
        }
        guard let head = try? AHJSON.decoder.decode(Plain.self, from: plaintext.subdata(in: 5..<5 + headerLength)) else {
            throw EnvelopeError.malformed
        }
        let bytes = plaintext.subdata(in: 5 + headerLength..<plaintext.count)
        guard bytes.count <= Wire.assetChunkBytes else { throw EnvelopeError.malformed }
        return (head, bytes)
    }

    /// Builds the wire bytes of a blob frame: a binary envelope around `encodeBlob`, never compressed. `nil` when the
    /// sender or `kid` does not fit the binary header (`EnvelopeFrame.encodeBinary`), when the header or the bytes are
    /// too long, or for a push; the caller then sends the same message as JSON.
    public static func sealBlob(
        key: Data, scope: EnvelopeScope, sender: String, kid: Int, head: Plain, bytes: Data
    ) -> Data? {
        sealBlob(
            key: key, scope: scope, sender: sender, kid: kid, nonce: Bytes.random(AHCrypto.nonceLength), head: head,
            bytes: bytes
        )
    }

    /// `sealBlob` with an explicit nonce (fixtures and tests).
    public static func sealBlob(
        key: Data, scope: EnvelopeScope, sender: String, kid: Int, nonce: Data, head: Plain, bytes: Data
    ) -> Data? {
        guard scope != .push, let plaintext = try? encodeBlob(head: head, bytes: bytes) else { return nil }
        return sealFrame(key: key, scope: scope, sender: sender, kid: kid, nonce: nonce, plaintext: plaintext)
            .encodeBinary()
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

    /// The header of a blob frame (`AHEnvelope.sealBlob`): the message as it goes in JSON, without its `data`, because
    /// the bytes follow the header instead. Pass the body with an empty `data`.
    public init<M: WireMessage>(blobHeader body: M, nowMs: Int64) throws {
        var tree = try AHJSON.tree(body)
        if case .object(var object) = tree {
            object["data"] = nil
            tree = .object(object)
        }
        self.init(t: M.type, ts: nowMs, b: tree)
    }

    /// Decodes the body as `M`; fails when the type string or the shape does not match.
    public func body<M: WireMessage>(as type: M.Type) throws -> M {
        guard t == M.type else { throw EnvelopeError.malformed }
        return try AHJSON.value(M.self, from: b)
    }

    /// Builds the wire bytes for one message, in the JSON envelope unless the caller says the peer opens the binary one
    /// (and the sender and `kid` fit its header, and the message does not go in the push scope).
    public func sealed(
        key: Data, scope: EnvelopeScope, sender: String, kid: Int, format: WireFormat = .json
    ) throws -> Data {
        let plaintext = try AHJSON.encoder.encode(self)
        let frame = AHEnvelope.sealFrame(
            key: key, scope: scope, sender: sender, kid: kid, nonce: Bytes.random(AHCrypto.nonceLength),
            plaintext: plaintext
        )
        return AHEnvelope.encode(frame, scope: scope, format: format)
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
