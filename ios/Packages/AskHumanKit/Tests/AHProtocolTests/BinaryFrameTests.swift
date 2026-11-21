import XCTest
@testable import AHProtocol

/// The binary envelope and the blob frame (docs/specs/ios-attachment-traffic.md §5), beyond what the cross-language
/// fixtures pin: the layout, the strict reading, and the cases that fall back to JSON.
final class BinaryFrameTests: XCTestCase {
    private let key = Data(repeating: 7, count: 32)
    private let mac = "mfx2c3rhs6kq7tw4dnb5vypa2e"
    private let topic = "ah/askhuman01/d/3qjxfkln6hdpmcvbtzaswe4r5y"
    private let nonce = Data((0..<12).map { UInt8($0 + 1) })

    private func ping(_ body: JSONValue = .object(["t": .int(1)])) -> Plain {
        Plain(i: "AAAAAAAAAAAAAAAAAAAAAA", t: "ping", ts: 1_792_900_000_000, b: body)
    }

    private func chunkHead(index: UInt64 = 1) throws -> Plain {
        var head = try Plain(
            blobHeader: AssetChunk(assetId: "a1", index: index, total: 3, size: 500_000, sha256: nil, data: ""),
            nowMs: 1_792_900_000_000
        )
        head.i = "BBBBBBBBBBBBBBBBBBBBBB"
        return head
    }

    private func sealed(_ plain: Plain, kid: Int = Wire.kidPair, sender: String? = nil, format: WireFormat = .binary) throws -> Data {
        try plain.sealed(key: key, scope: .topic(topic), sender: sender ?? mac, kid: kid, format: format)
    }

    private func opened(_ wire: Data, scope: EnvelopeScope? = nil) throws -> OpenedMessage {
        try AHEnvelope.openMessage(key: key, scope: scope ?? .topic(topic), frame: AHEnvelope.parseFrame(wire))
    }

    /// Opens, whatever the failure was, and says what it was.
    private func failure(_ wire: Data, scope: EnvelopeScope? = nil) -> EnvelopeError? {
        do {
            _ = try opened(wire, scope: scope)
            return nil
        } catch {
            return error as? EnvelopeError
        }
    }

    // MARK: ids

    func testOnlyAnIdWrittenTheWayNewIdWritesItConvertsToBytes() throws {
        let bytes = try XCTUnwrap(Bytes.idToBytes(mac))
        XCTAssertEqual(bytes.count, 16)
        XCTAssertEqual(Bytes.base32Lower(bytes), mac)
        for _ in 0..<200 {
            let id = Bytes.newId()
            XCTAssertEqual(Bytes.idToBytes(id).map(Bytes.base32Lower), id)
        }
        // A valid id as far as the alphabet and the length go, whose last character carries two bits that are not zero:
        // another text for the same bytes.
        let padded = String(repeating: "a", count: 25) + "b"
        XCTAssertTrue(Bytes.isValidId(padded))
        XCTAssertNil(Bytes.idToBytes(padded))
        for bad in ["", "a", String(mac.dropLast()), mac + "a", mac.uppercased(), String(mac.dropLast()) + "0", String(mac.dropLast()) + "1", String(mac.dropLast()) + "8", "名" + String(mac.dropFirst())] {
            XCTAssertNil(Bytes.idToBytes(bad), bad)
        }
    }

    func testBase32DecodesWhatItEncodesAndNothingElse() throws {
        let fx = try Fixtures.json("crypto.json")
        for item in fx["base32"] as! [[String: String]] {
            let bytes = Bytes.fromHex(item["hex"]!)!
            XCTAssertEqual(Bytes.base32LowerDecode(item["text"]!), bytes, item["text"]!)
        }
        XCTAssertEqual(Bytes.base32LowerDecode(""), Data())
        // Five characters are one byte and a character too many; the unused bits must be zero.
        XCTAssertNil(Bytes.base32LowerDecode("a"))
        XCTAssertNil(Bytes.base32LowerDecode("ab"))
        XCTAssertEqual(Bytes.base32LowerDecode("aa"), Data([0]))
        XCTAssertNil(Bytes.base32LowerDecode("a="))
        XCTAssertNil(Bytes.base32LowerDecode("A"))
    }

    // MARK: the binary envelope

    func testTheBinaryEnvelopeHasTheDocumentedLayout() throws {
        let message = ping()
        let plaintext = try AHJSON.encoder.encode(message)
        let frame = AHEnvelope.sealFrame(
            key: key, scope: .topic(topic), sender: mac, kid: 258, nonce: nonce, plaintext: plaintext
        )
        let wire = try XCTUnwrap(frame.encodeBinary())
        XCTAssertEqual(wire[0], 0xA1, "the magic")
        XCTAssertEqual(wire[1], 0, "no flag")
        XCTAssertEqual(Array(wire[2..<4]), [0x01, 0x02], "the key id, big endian")
        XCTAssertEqual(wire.subdata(in: 4..<20), Bytes.idToBytes(mac), "the sender as raw bytes")
        XCTAssertEqual(wire.subdata(in: 20..<32), nonce)
        XCTAssertEqual(wire.count, 32 + plaintext.count + 16, "the header, the ciphertext, the tag")

        let parsed = try AHEnvelope.parseFrame(wire)
        XCTAssertEqual(parsed, EnvelopeFrame(
            sender: mac, kid: 258, nonce: nonce, ciphertext: frame.ciphertext, compressed: false, binary: true
        ))
        // It is the same ciphertext a JSON envelope carries: the format changes nothing that is signed.
        XCTAssertEqual(parsed.envelope.c, Bytes.b64u(frame.ciphertext))
        XCTAssertEqual(
            try AHEnvelope.openFrame(key: key, scope: .topic(topic), frame: EnvelopeFrame(parsed.envelope)), plaintext
        )
    }

    func testItTakesFarFewerBytesThanTheJSONEnvelope() throws {
        let message = ping()
        let binary = try sealed(message)
        let json = try sealed(message, format: .json)
        XCTAssertEqual(binary.count, 48 + (try AHJSON.encoder.encode(message)).count)
        XCTAssertLessThan(binary.count * 4, json.count * 3, "less than three quarters of the JSON form")
    }

    func testBothFormatsOpenToTheSameMessage() throws {
        let message = ping(.object(["text": .string("caf\u{e9} \u{91cd}\u{6784}")]))
        XCTAssertEqual(try opened(sealed(message)), .message(message))
        XCTAssertEqual(try opened(sealed(message, format: .json)), .message(message))
        // A JSON envelope starts with `{`, a binary one with a byte that is not ASCII.
        XCTAssertEqual(try sealed(message, format: .json).first, UInt8(ascii: "{"))
        XCTAssertEqual(try sealed(message).first, AHEnvelope.binaryMagic)
        XCTAssertEqual(try Plain.open(key: key, scope: .topic(topic), envelope: AHEnvelope.parse(sealed(message, format: .json))), message)
    }

    func testTheHeaderIsBoundToTheCiphertext() throws {
        let wire = try sealed(ping())
        XCTAssertNil(failure(wire))
        for index in 1..<wire.count {
            var tampered = wire
            tampered[index] ^= 0x01
            XCTAssertNotNil(failure(tampered), "byte \(index)")
        }
        XCTAssertEqual(failure(wire, scope: .topic(topic + "x")), .auth)
        XCTAssertEqual(failure(wire, scope: .push), .auth)
    }

    /// The sender or the key id cannot be written in the header, the message is in the push scope, or nobody said the
    /// peer opens binary: the same message goes as JSON, which every peer reads.
    func testAMessageThatDoesNotFitTheHeaderGoesAsJSON() throws {
        let message = ping()
        XCTAssertEqual(try sealed(message, kid: 65_535).first, AHEnvelope.binaryMagic, "the largest key id that fits")
        XCTAssertEqual(try sealed(message, kid: 0).first, AHEnvelope.binaryMagic)
        for wire in [
            try sealed(message, kid: 65_536),
            try sealed(message, kid: -1),
            try sealed(message, sender: "someone"),
            try sealed(message, sender: String(repeating: "a", count: 25) + "b"),
            try sealed(message, format: .json),
            try message.sealed(key: key, scope: .topic(topic), sender: mac, kid: 1),
            try message.sealed(key: key, scope: .push, sender: mac, kid: 1, format: .binary),
        ] {
            XCTAssertEqual(wire.first, UInt8(ascii: "{"))
        }
        XCTAssertEqual(try opened(sealed(message, kid: 65_536)), .message(message), "and it still opens")
    }

    func testTheBinaryEnvelopeIsReadStrictly() throws {
        let wire = try sealed(ping())
        XCTAssertThrowsError(try AHEnvelope.parseFrame(wire.prefix(47)), "too short for a tag") {
            XCTAssertEqual($0 as? EnvelopeError, .malformed)
        }
        XCTAssertThrowsError(try AHEnvelope.parseFrame(Data([0xA1])), "the magic alone") {
            XCTAssertEqual($0 as? EnvelopeError, .malformed)
        }
        // The shortest envelope is the header and a tag; it reads, and opens to nothing that is a message.
        let empty = try XCTUnwrap(
            AHEnvelope.sealFrame(key: key, scope: .topic(topic), sender: mac, kid: 1, nonce: nonce, plaintext: Data()).encodeBinary()
        )
        XCTAssertEqual(empty.count, 48)
        XCTAssertNoThrow(try AHEnvelope.parseFrame(empty))
        XCTAssertEqual(failure(empty), .malformed)

        for bit in 1..<8 {
            var reserved = wire
            reserved[1] = UInt8(1 << bit)
            XCTAssertThrowsError(try AHEnvelope.parseFrame(reserved), "reserved flag \(bit)") {
                XCTAssertEqual($0 as? EnvelopeError, .malformed)
            }
        }
        var tooLong = Data([AHEnvelope.binaryMagic])
        tooLong.append(Data(count: Wire.maxEnvelopeBytes))
        XCTAssertThrowsError(try AHEnvelope.parseFrame(tooLong)) { XCTAssertEqual($0 as? EnvelopeError, .tooLarge) }

        // Anything that is not the magic is read as JSON, so a damaged binary envelope is not mistaken for a JSON one.
        var otherMagic = wire
        otherMagic[0] = 0xA2
        XCTAssertThrowsError(try AHEnvelope.parseFrame(otherMagic)) { XCTAssertEqual($0 as? EnvelopeError, .malformed) }

        // A slice of a larger buffer reads the same.
        let slice = (Data([9, 9, 9]) + wire).dropFirst(3)
        XCTAssertNotEqual(slice.startIndex, 0)
        XCTAssertEqual(try AHEnvelope.parseFrame(slice), try AHEnvelope.parseFrame(wire))
        XCTAssertEqual(try opened(Data(slice)), try opened(wire))
    }

    func testACompressedBinaryEnvelopeInflatesAndItsBitIsAuthenticated() throws {
        let message = ping(.object(["text": .string(String(repeating: "a stable title, ", count: 80))]))
        let plaintext = try AHJSON.encoder.encode(message)
        let frame = AHEnvelope.sealFrame(
            key: key, scope: .topic(topic), sender: mac, kid: 1, nonce: nonce,
            plaintext: AHEnvelope.compress(plaintext), compressed: true
        )
        let wire = try XCTUnwrap(frame.encodeBinary())
        XCTAssertEqual(wire[1], 1)
        XCTAssertLessThan(wire.count * 4, plaintext.count)
        XCTAssertEqual(try opened(wire), .message(message))

        var unmarked = wire
        unmarked[1] = 0
        XCTAssertEqual(failure(unmarked), .auth, "the marker is part of what is authenticated")
        XCTAssertEqual(failure(wire, scope: .push), .malformed, "a push is never compressed")
    }

    // MARK: blob frames

    func testABlobFrameCarriesTheBytesAsTheyAreAfterItsHeader() throws {
        let head = try chunkHead()
        let bytes = Data((0..<256).map { UInt8($0) }) + Bytes.random(5_000)
        let wire = try XCTUnwrap(AHEnvelope.sealBlob(key: key, scope: .topic(topic), sender: mac, kid: 1, nonce: nonce, head: head, bytes: bytes))
        XCTAssertEqual(try opened(wire), .blob(head: head, bytes: bytes))

        let frame = try AHEnvelope.parseFrame(wire)
        let plaintext = try AHEnvelope.openFrame(key: key, scope: .topic(topic), frame: frame)
        XCTAssertEqual(plaintext.first, AHEnvelope.blobMarker)
        XCTAssertTrue(plaintext.suffix(bytes.count).elementsEqual(bytes), "the bytes are in the plaintext as they are")
        let headerLength = (1..<5).reduce(0) { $0 << 8 | Int(plaintext[$1]) }
        XCTAssertEqual(try AHJSON.decoder.decode(Plain.self, from: plaintext.subdata(in: 5..<5 + headerLength)), head)
        XCTAssertEqual(plaintext.count, 5 + headerLength + bytes.count)
        XCTAssertEqual(wire.count, 32 + plaintext.count + 16)
        XCTAssertLessThan(headerLength, 400, "a header is a few hundred bytes at most")
    }

    func testAFullChunkTakesFarFewerBytesAsABlobFrameThanAsJSON() throws {
        let bytes = Bytes.random(Wire.assetChunkBytes)
        let blob = try XCTUnwrap(AHEnvelope.sealBlob(key: key, scope: .topic(topic), sender: mac, kid: 1, head: chunkHead(), bytes: bytes))
        var body = try chunkHead()
        body.b = try AHJSON.tree(AssetChunk(assetId: "a1", index: 1, total: 3, size: 500_000, sha256: nil, data: Bytes.b64u(bytes)))
        let json = try sealed(body, format: .json)
        XCTAssertLessThan(blob.count, Wire.maxEnvelopeBytes)
        XCTAssertLessThan(blob.count * 10, json.count * 6, "less than 60% of the JSON form (it is base64 twice there)")
        XCTAssertLessThan(blob.count, bytes.count + 1_000, "the file's bytes and less than a kilobyte around them")
    }

    func testABlobHeaderAndItsBytesAreBounded() throws {
        let head = try chunkHead()
        let full = Bytes.random(Wire.assetChunkBytes)
        XCTAssertNoThrow(try AHEnvelope.encodeBlob(head: head, bytes: full))
        XCTAssertNoThrow(try AHEnvelope.encodeBlob(head: head, bytes: Data()))
        XCTAssertThrowsError(try AHEnvelope.encodeBlob(head: head, bytes: full + [0])) {
            XCTAssertEqual($0 as? EnvelopeError, .tooLarge)
        }
        var long = head
        long.b = .object(["note": .string(String(repeating: "x", count: Wire.blobHeaderMaxBytes))])
        XCTAssertThrowsError(try AHEnvelope.encodeBlob(head: long, bytes: Data([1]))) {
            XCTAssertEqual($0 as? EnvelopeError, .tooLarge)
        }
        XCTAssertNil(AHEnvelope.sealBlob(key: key, scope: .topic(topic), sender: mac, kid: 1, head: long, bytes: Data([1])))
        XCTAssertNil(AHEnvelope.sealBlob(key: key, scope: .topic(topic), sender: mac, kid: 1, head: head, bytes: full + [0]))
        // And what it cannot be sealed as is left for the caller to send as JSON.
        XCTAssertNil(AHEnvelope.sealBlob(key: key, scope: .push, sender: mac, kid: 1, head: head, bytes: Data([1])))
        XCTAssertNil(AHEnvelope.sealBlob(key: key, scope: .topic(topic), sender: mac, kid: 65_536, head: head, bytes: Data([1])))
        XCTAssertNil(AHEnvelope.sealBlob(key: key, scope: .topic(topic), sender: "someone", kid: 1, head: head, bytes: Data([1])))
    }

    func testAMalformedBlobPlaintextIsRefusedEvenWhenItAuthenticates() throws {
        let head = try AHJSON.encoder.encode(chunkHead())
        func blob(declared: Int, header: Data, tail: Data) -> Data {
            var out = Data([AHEnvelope.blobMarker])
            out.append(contentsOf: withUnsafeBytes(of: UInt32(declared).bigEndian) { Array($0) })
            out.append(header)
            out.append(tail)
            return out
        }
        let cases: [(String, Data)] = [
            ("the marker alone", Data([0x01])),
            ("a cut length", Data([0x01, 0, 0, 0])),
            ("a length past the end", blob(declared: head.count + 1, header: head, tail: Data())),
            ("a length over the limit", blob(declared: Wire.blobHeaderMaxBytes + 1, header: Data("{}".utf8), tail: Data(count: 20))),
            ("a header that is not JSON", blob(declared: 5, header: Data("nope!".utf8), tail: Data("abc".utf8))),
            ("a header that is not a message", blob(declared: 2, header: Data("{}".utf8), tail: Data("abc".utf8))),
            ("more bytes than a chunk", blob(declared: head.count, header: head, tail: Data(count: Wire.assetChunkBytes + 1))),
            ("a plaintext over the limit", blob(declared: head.count, header: head, tail: Data(count: Wire.maxPlainBytes))),
            ("neither JSON nor a blob", Data([0x02, 0x7b, 0x7d])),
        ]
        for (name, plaintext) in cases {
            XCTAssertThrowsError(try AHEnvelope.decodeBlob(plaintext), name)
            let frame = AHEnvelope.sealFrame(key: key, scope: .topic(topic), sender: mac, kid: 1, nonce: nonce, plaintext: plaintext)
            let wire = try XCTUnwrap(frame.encodeBinary())
            XCTAssertEqual(failure(wire), .malformed, name)
        }
        // The same plaintext with a good length is a blob.
        let good = blob(declared: head.count, header: head, tail: Data("abc".utf8))
        XCTAssertEqual(try AHEnvelope.decodeBlob(good).bytes, Data("abc".utf8))
        XCTAssertEqual(try AHEnvelope.decodeBlob((Data([5]) + good).dropFirst()).bytes, Data("abc".utf8), "from a slice")
    }

    func testABlobIsBoundToItsTopicAndIsNeverCompressedOrAPush() throws {
        let head = try chunkHead()
        let bytes = Bytes.random(2_000)
        let wire = try XCTUnwrap(AHEnvelope.sealBlob(key: key, scope: .topic(topic), sender: mac, kid: 1, head: head, bytes: bytes))
        XCTAssertEqual(failure(wire, scope: .topic(topic + "x")), .auth)
        XCTAssertEqual(failure(wire, scope: .push), .auth)

        // A blob plaintext inside a compressed envelope: it authenticates and inflates, and is still refused.
        let plaintext = try AHEnvelope.encodeBlob(head: head, bytes: bytes)
        let compressed = AHEnvelope.sealFrame(
            key: key, scope: .topic(topic), sender: mac, kid: 1, nonce: nonce,
            plaintext: AHEnvelope.compress(plaintext), compressed: true
        )
        XCTAssertEqual(failure(try XCTUnwrap(compressed.encodeBinary())), .malformed)
        XCTAssertEqual(failure(AHEnvelope.encode(compressed.envelope)), .malformed)

        // Nor in the push scope, where an envelope is JSON in any case.
        let push = AHEnvelope.sealFrame(key: key, scope: .push, sender: mac, kid: 1, nonce: nonce, plaintext: plaintext)
        XCTAssertEqual(failure(try XCTUnwrap(push.encodeBinary()), scope: .push), .malformed)
    }

    // MARK: the messages

    func testTheBinaryCapabilityIsOnTheWireOnlyWhenOn() throws {
        let off = Hello(nonce: "n", appVer: "1", deviceName: "d")
        XCTAssertNil(try AHJSON.tree(off)["binary"])
        let on = Hello(nonce: "n", appVer: "1", deviceName: "d", binary: true)
        XCTAssertEqual(try AHJSON.tree(on)["binary"], .bool(true))
        XCTAssertFalse(try AHJSON.decoder.decode(Hello.self, from: Data(#"{"nonce":"n","proto":1,"appVer":"1","deviceName":"d"}"#.utf8)).binary)

        XCTAssertNil(try AHJSON.tree(Caps(newTask: true, interject: true, watch: true))["binary"])
        XCTAssertEqual(try AHJSON.tree(Caps(newTask: true, interject: true, watch: true, binary: true))["binary"], .bool(true))
        let old = try AHJSON.decoder.decode(Caps.self, from: Data(#"{"newTask":true,"interject":true,"watch":true}"#.utf8))
        XCTAssertFalse(old.binary)
    }

    /// The header of a blob frame is the message without its `data`; the message itself always has the key, an empty
    /// file's one chunk included, because older peers do not read a chunk that lacks it.
    func testAChunkHeaderHasNoDataAndTheMessageAlwaysKeepsIt() throws {
        let chunk = AssetChunk(assetId: "a1", index: 0, total: 1, size: 0, sha256: nil, data: "")
        XCTAssertEqual(try AHJSON.tree(chunk)["data"], .string(""), "an empty file's chunk keeps an empty key")
        let head = try Plain(blobHeader: chunk, nowMs: 5)
        XCTAssertEqual(head.t, AssetChunk.type)
        XCTAssertNil(head.b["data"], "the header leaves it out")
        XCTAssertEqual(head.b["assetId"], .string("a1"))
        let decoded = try head.body(as: AssetChunk.self)
        XCTAssertEqual(decoded, chunk, "and it reads as empty")

        let full = try AHJSON.tree(AssetChunk(assetId: "a1", index: 0, total: 1, size: 3, sha256: nil, data: "AQID"))
        XCTAssertEqual(full["data"], .string("AQID"))
        XCTAssertEqual(try AHJSON.value(AssetChunk.self, from: full).data, "AQID")
        XCTAssertNil(try Plain(blobHeader: AssetChunk(assetId: "a1", index: 0, total: 1, size: 3, sha256: nil, data: "AQID"), nowMs: 5).b["data"])

        let upload = UploadChunk(uploadId: "u1", index: 0, data: "")
        XCTAssertEqual(try AHJSON.tree(upload)["data"], .string(""))
        let uploadHead = try Plain(blobHeader: upload, nowMs: 5)
        XCTAssertNil(uploadHead.b["data"])
        XCTAssertEqual(try uploadHead.body(as: UploadChunk.self), upload)
        XCTAssertEqual(try AHJSON.tree(UploadChunk(uploadId: "u1", index: 0, data: "AQID"))["data"], .string("AQID"))
    }
}
