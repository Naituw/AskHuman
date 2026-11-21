import XCTest
import CryptoKit
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The phone's side of docs/specs/ios-attachment-traffic.md §5: it says in `hello` that it opens binary envelopes and
/// blob frames, sends binary only once the `hello.ack` of the running session says the Mac does too, and reads either
/// format from the Mac at any time. `TestMac` is the Mac of the real build (`caps.binary` on) unless a test turns that off.
final class BinaryTransportTests: XCTestCase {
    private func randomData(_ count: Int) -> Data { Data((0..<count).map { _ in UInt8.random(in: 0...255) }) }

    private func asset(size: Int) -> AssetRef {
        AssetRef(assetId: Bytes.newId(), name: "file.bin", size: UInt64(size), mime: "application/octet-stream", isImage: false, tooLarge: false)
    }

    /// A request in the inbox, which uploads need.
    private func pairedWithRequest(_ h: Harness) async -> RequestView {
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        return request
    }

    private func upload(_ h: Harness, _ request: RequestView, _ data: Data) async throws -> String {
        try await h.engine.upload(
            macId: h.mac.macId, requestId: request.id,
            source: UploadSource(name: "log.bin", mime: "application/octet-stream", data: data, isImage: false)
        ) { _ in }
    }

    // MARK: negotiation

    func testTheHelloIsJSONAndSaysThePhoneOpensBinary() async throws {
        let h = Harness()
        await h.pair()
        XCTAssertEqual(h.mac.hello(of: h.phoneId)?.binary, true)
        let hellos = h.mac.arrivals(Wire.T.hello)
        XCTAssertFalse(hellos.isEmpty)
        XCTAssertTrue(hellos.allSatisfy { !$0.binary && !$0.blob }, "the Mac cannot know yet that it may be sent binary")
        let caps = await h.state().macs.first?.caps
        XCTAssertEqual(caps?.binary, true, "the ack said it")
    }

    func testTheEngineSendsJSONUntilTheAckOfTheSessionAndBinaryAfterIt() async throws {
        let h = Harness()
        await h.pair()
        let macId = h.mac.macId
        await h.engine.send(Ping(t: 1), to: macId)
        await eventually("the first ping") { h.mac.count(Ping.type) == 1 }
        XCTAssertEqual(h.mac.arrivals(Ping.type).map(\.binary), [true], "the Mac said in its ack that it opens binary")

        // The Mac comes back as a new session and has not answered the new `hello` yet.
        h.mac.autoHello = false
        await h.mac.publishPresence(online: false)
        await eventually("the phone to see the Mac leave") { await h.state().macs.first?.link == .macOffline }
        h.mac.epoch = "epoch-restarted"
        await h.mac.publishPresence(online: true)
        await eventually("the new hello") { h.mac.count(Wire.T.hello) == 2 }
        // The clock stands still, so the send bucket (a burst of ten) refills only when it is moved: two seconds are less
        // than the five a hello waits for its ack.
        await h.clock.advance(by: 2)
        await h.engine.send(Ping(t: 2), to: macId)
        await eventually("the second ping") { h.mac.count(Ping.type) == 2 }
        XCTAssertEqual(h.mac.arrivals(Ping.type).map(\.binary), [true, false], "nothing is binary until the ack of this session")
        XCTAssertEqual(h.mac.arrivals(Wire.T.hello).map(\.binary), [false, false])

        await h.mac.sendHelloSync(to: h.phoneId, nonce: try unwrap(h.mac.hello(of: h.phoneId)).nonce)
        await eventually("the new session") { await h.state().macs.first?.link == .online }
        await h.clock.advance(by: 2)
        await h.engine.send(Ping(t: 3), to: macId)
        await eventually("the third ping") { h.mac.count(Ping.type) == 3 }
        XCTAssertEqual(h.mac.arrivals(Ping.type).map(\.binary), [true, false, true])
    }

    func testAMacThatComesBackWithoutBinaryIsSentJSONAgain() async throws {
        let h = Harness()
        await h.pair()
        let macId = h.mac.macId
        await h.engine.send(Ping(t: 1), to: macId)
        await eventually("the first ping") { h.mac.count(Ping.type) == 1 }

        // The Mac restarts onto an older build: what the first ack said does not carry over.
        await h.mac.publishPresence(online: false)
        await eventually("the phone to see the Mac leave") { await h.state().macs.first?.link == .macOffline }
        h.mac.caps.binary = false
        h.mac.epoch = "epoch-older-build"
        await h.mac.publishPresence(online: true)
        await eventually("the new session") {
            let link = await h.state().macs.first?.link
            return h.mac.count(Wire.T.hello) == 2 && link == .online
        }
        await h.clock.advance(by: 2)
        await h.engine.send(Ping(t: 2), to: macId)
        await eventually("the second ping, which this Mac could read") { h.mac.count(Ping.type) == 2 }
        XCTAssertEqual(h.mac.arrivals(Ping.type).map(\.binary), [true, false])
    }

    func testAMacThatDoesNotDeclareBinaryIsSentJSONOnly() async throws {
        let h = Harness()
        h.mac.caps.binary = false
        let request = await pairedWithRequest(h)
        XCTAssertEqual(h.mac.hello(of: h.phoneId)?.binary, true, "the phone says it all the same")
        let caps = await h.state().macs.first?.caps
        XCTAssertEqual(caps?.binary, false)

        // This Mac reads the JSON envelope only, so everything that works here went as JSON.
        await h.engine.send(Ping(t: 1), to: h.mac.macId)
        await eventually("a ping that this Mac could read") { h.mac.count(Ping.type) == 1 }
        // The failure is recorded; the rest would only wait for acknowledgements that cannot come.
        guard h.mac.count(Ping.type) == 1 else { return }
        let data = randomData(Wire.assetChunkBytes * 2 + 5)
        let id = try await upload(h, request, data)
        XCTAssertEqual(h.mac.uploaded(id)?.data, data)
        let chunks = h.mac.arrivals(UploadChunk.type)
        XCTAssertEqual(chunks.count, 3)
        XCTAssertTrue(chunks.allSatisfy { !$0.binary && !$0.blob })

        let ref = asset(size: data.count)
        h.mac.register(asset: ref.assetId, data: data)
        let url = try await h.engine.download(macId: h.mac.macId, asset: ref) { _ in }
        XCTAssertEqual(try Data(contentsOf: url), data)
        XCTAssertEqual(h.mac.binarySent, 0)
    }

    // MARK: uploads and downloads

    func testUploadChunksGoAsBlobFramesToAMacThatTakesBinary() async throws {
        let h = Harness()
        let request = await pairedWithRequest(h)
        let data = randomData(Wire.assetChunkBytes * 5 + 77)
        let id = try await upload(h, request, data)
        XCTAssertEqual(h.mac.uploaded(id)?.data, data)
        let blob = Arrival(type: UploadChunk.type, binary: true, blob: true)
        XCTAssertEqual(h.mac.arrivals(UploadChunk.type), Array(repeating: blob, count: 6))
        XCTAssertEqual(h.mac.arrivals(UploadBegin.type), [Arrival(type: UploadBegin.type, binary: true, blob: false)])

        // The clock stands still, so the send bucket (a burst of ten) refills only when it is moved.
        await h.clock.advance(by: 2)
        let small = try await upload(h, request, Data([1, 2, 3]))
        XCTAssertEqual(h.mac.uploaded(small)?.data, Data([1, 2, 3]))
        XCTAssertEqual(h.mac.arrivals(UploadChunk.type).last, blob)
    }

    func testADownloadArrivesInBlobFrames() async throws {
        let h = Harness()
        await h.pair()
        let data = randomData(Wire.assetChunkBytes * 19 - 1234)
        let ref = asset(size: data.count)
        h.mac.register(asset: ref.assetId, data: data)
        let seen = Counter()
        let url = try await h.engine.download(macId: h.mac.macId, asset: ref) { seen.record($0) }
        XCTAssertEqual(try Data(contentsOf: url), data)
        XCTAssertEqual(h.mac.blobsSent, 19)
        XCTAssertEqual(seen.values, seen.values.sorted(), "progress never goes backwards")
        XCTAssertEqual(seen.last, 1)
    }

    func testAChunkAsAJSONMessageInTheBinaryEnvelopeIsReadToo() async throws {
        let h = Harness()
        h.mac.blobChunks = false
        await h.pair()
        let data = randomData(Wire.assetChunkBytes * 2 + 9)
        let ref = asset(size: data.count)
        h.mac.register(asset: ref.assetId, data: data)
        let url = try await h.engine.download(macId: h.mac.macId, asset: ref) { _ in }
        XCTAssertEqual(try Data(contentsOf: url), data)
        XCTAssertEqual(h.mac.blobsSent, 0)
        XCTAssertGreaterThan(h.mac.binarySent, 0, "everything else went binary")
    }

    /// The one chunk of a zero-byte file has no bytes: as a blob frame that is a header and nothing after it, and as a JSON
    /// message it is `data: ""`, which has to be there for a phone of an older build to read it.
    func testAnEmptyFileIsDownloadedWhicheverWayItsChunkComes() async throws {
        for (label, caps, blobs) in [("blob frame", true, true), ("JSON in binary", true, false), ("older Mac", false, false)] {
            let h = Harness()
            h.mac.caps.binary = caps
            h.mac.blobChunks = blobs
            await h.pair()
            await h.engine.send(Ping(t: 1), to: h.mac.macId)
            await eventually("a ping that this Mac could read (\(label))") { h.mac.count(Ping.type) == 1 }
            // The failure is recorded; the rest would only wait for a window the Mac never saw.
            guard h.mac.count(Ping.type) == 1 else { return }
            let ref = asset(size: 0)
            h.mac.register(asset: ref.assetId, data: Data())
            let url = try await h.engine.download(macId: h.mac.macId, asset: ref) { _ in }
            XCTAssertEqual(try Data(contentsOf: url), Data(), label)
            XCTAssertEqual(h.mac.blobsSent, blobs ? 1 : 0, label)
        }
    }

    // MARK: what is refused

    func testFramesThatAreNotValidAreDroppedAndTheStreamGoesOn() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        let keys = try unwrap(h.mac.downKeys(for: h.phoneId))
        let now = h.clock.nowMs()
        let macId = h.mac.macId
        func blob(_ head: Plain, scope: EnvelopeScope? = nil, key: Data? = nil) throws -> Data {
            try unwrap(AHEnvelope.sealBlob(
                key: key ?? keys.m2p, scope: scope ?? .topic(keys.topic), sender: macId, kid: keys.kid, head: head,
                bytes: Data([1, 2, 3])
            ))
        }

        // A download that waits for its chunk, which the forged frames below try to supply with other bytes.
        let real = Data([9, 9, 9])
        let ref = asset(size: real.count)
        h.mac.register(asset: ref.assetId, data: real)
        h.mac.dropAssetGets = 1
        let download = Task { try await h.engine.download(macId: macId, asset: ref) { _ in } }
        await eventually("the get") { h.mac.count(AssetGet.type) == 1 }

        // The very change the phone would apply next, so that a frame that got through would show.
        let resolved = RequestResolved(rev: h.mac.inboxRevision + 1, requestId: request.id, how: "answeredElsewhere", by: "Telegram")
        let twice = AssetChunk(assetId: ref.assetId, index: 0, total: 1, size: 3, sha256: nil, data: "AQID")
        let chunk = AssetChunk(assetId: ref.assetId, index: 0, total: 1, size: 3, sha256: nil, data: "")
        let good = try blob(Plain(blobHeader: chunk, nowMs: now))
        var flipped = good
        flipped[flipped.count - 5] ^= 0x01
        let forged: [(String, Data)] = [
            // Only a chunk of an asset comes as a blob: resolving the request would take it out of the inbox.
            ("a blob of another message", try blob(Plain(resolved, nowMs: now))),
            // The bytes ride behind the header: a header that says them too is two messages in one.
            ("a header with its own data", try blob(Plain(twice, nowMs: now))),
            ("a frame for another topic", try blob(Plain(blobHeader: chunk, nowMs: now), scope: .topic(keys.topic + "x"))),
            ("a frame under another key", try blob(Plain(blobHeader: chunk, nowMs: now), key: Bytes.random(32))),
            ("a flipped byte", flipped),
            ("a cut frame", good.prefix(30)),
            ("the magic alone", Data([AHEnvelope.binaryMagic])),
        ]
        for (_, bytes) in forged { await h.mac.sendBytes(bytes, to: h.phoneId) }
        try await Task.sleep(nanoseconds: 150_000_000)

        let inbox = await h.state().inbox
        XCTAssertEqual(inbox.map(\.requestId), [request.id], "none of them resolved the request: \(forged.map(\.0))")
        let link = await h.state().macs.first?.link
        XCTAssertEqual(link, .online)

        // The download was not completed by any of them: the retry gets the real file.
        await h.advance(until: { h.mac.count(AssetGet.type) == 2 }, "the retry of the window")
        let url = try await download.value
        XCTAssertEqual(try Data(contentsOf: url), real)

        // The stream goes on in both formats: a message in the binary envelope, and one in the JSON envelope.
        await h.mac.deliver(sampleAsk())
        await eventually("the second request") { await h.state().inbox.count == 2 }
        await h.mac.sendRaw(plain: try Plain(MacInfo(macName: "Renamed Mac"), nowMs: now), to: h.phoneId)
        await eventually("the JSON envelope renaming the Mac") { await h.state().macs.first?.name == "Renamed Mac" }
    }
}
