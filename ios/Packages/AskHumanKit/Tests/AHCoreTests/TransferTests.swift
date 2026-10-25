import XCTest
import CryptoKit
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

final class TransferTests: XCTestCase {
    private func asset(_ id: String = Bytes.newId(), size: Int, tooLarge: Bool = false) -> AssetRef {
        AssetRef(assetId: id, name: "file.bin", size: UInt64(size), mime: "application/octet-stream", isImage: false, tooLarge: tooLarge)
    }

    private func randomData(_ count: Int) -> Data { Data((0..<count).map { _ in UInt8.random(in: 0...255) }) }

    /// A request in the inbox, which uploads need.
    private func pairedWithRequest(_ h: Harness) async -> RequestView {
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        return request
    }

    // MARK: downloads

    func testASmallAssetIsFetchedInOneChunkAndCached() async throws {
        let h = Harness()
        await h.pair()
        let data = randomData(10_000)
        let ref = asset(size: data.count)
        h.mac.register(asset: ref.assetId, data: data)

        let seen = Counter()
        let url = try await h.engine.download(macId: h.mac.macId, asset: ref) { seen.record($0) }
        XCTAssertEqual(try Data(contentsOf: url), data)
        XCTAssertEqual(seen.last, 1)

        let again = try await h.engine.download(macId: h.mac.macId, asset: ref) { _ in }
        XCTAssertEqual(again, url)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1, "the second call is served from the cache")
    }

    func testALargeAssetComesInWindowsOfEightChunks() async throws {
        let h = Harness()
        await h.pair()
        let data = randomData(Wire.assetChunkBytes * 19 - 1234)
        let ref = asset(size: data.count)
        h.mac.register(asset: ref.assetId, data: data)

        let seen = Counter()
        let url = try await h.engine.download(macId: h.mac.macId, asset: ref) { seen.record($0) }
        XCTAssertEqual(try Data(contentsOf: url), data)
        let gets = try h.mac.received(AssetGet.type).map { try $0.body(as: AssetGet.self) }
        XCTAssertEqual(gets.map(\.from), [0, 8, 16])
        XCTAssertTrue(gets.allSatisfy { $0.count == 8 })
        XCTAssertEqual(seen.values, seen.values.sorted(), "progress never goes backwards")
        XCTAssertEqual(seen.last, 1)
    }

    func testAChecksumMismatchFailsAndStoresNothing() async throws {
        let h = Harness()
        await h.pair()
        let data = randomData(5_000)
        let ref = asset(size: data.count)
        h.mac.register(asset: ref.assetId, data: data)
        h.mac.corruptAssetChecksum = true
        do {
            _ = try await h.engine.download(macId: h.mac.macId, asset: ref) { _ in }
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .invalid("Checksum mismatch."))
        }
        let path = await h.engine.cachedAssetURL(ref.assetId).path
        XCTAssertFalse(FileManager.default.fileExists(atPath: path))
    }

    func testAnUnknownAssetIsReportedByTheMac() async throws {
        let h = Harness()
        await h.pair()
        do {
            _ = try await h.engine.download(macId: h.mac.macId, asset: asset(size: 100)) { _ in }
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .unavailable("notFound"))
        }
    }

    func testAnAssetMarkedTooLargeIsNotRequested() async throws {
        let h = Harness()
        await h.pair()
        do {
            _ = try await h.engine.download(macId: h.mac.macId, asset: asset(size: 20_000_000, tooLarge: true)) { _ in }
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .tooLarge)
        }
        XCTAssertEqual(h.mac.count(AssetGet.type), 0)
    }

    func testDownloadWhileTheMacIsOfflineFailsAtOnce() async throws {
        let h = Harness()
        await h.pair()
        await h.mac.publishPresence(online: false)
        await eventually("offline") { await h.state().macs.first?.link == .macOffline }
        do {
            _ = try await h.engine.download(macId: h.mac.macId, asset: asset(size: 100)) { _ in }
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .macOffline)
        }
    }

    func testTwoWaitersShareOneTransfer() async throws {
        let h = Harness()
        await h.pair()
        let data = randomData(Wire.assetChunkBytes * 3)
        let ref = asset(size: data.count)
        h.mac.register(asset: ref.assetId, data: data)
        async let first = h.engine.download(macId: h.mac.macId, asset: ref) { _ in }
        async let second = h.engine.download(macId: h.mac.macId, asset: ref) { _ in }
        let urls = try await [first, second]
        XCTAssertEqual(urls[0], urls[1])
        XCTAssertEqual(h.mac.count(AssetGet.type), 1)
    }

    func testALostWindowIsAskedForAgainOnceThenTimesOut() async throws {
        let h = Harness()
        await h.pair()
        let ref = asset(size: 100)
        h.mac.register(asset: ref.assetId, data: randomData(100))
        h.mac.dropAssetGets = 2
        let task = Task { try await h.engine.download(macId: h.mac.macId, asset: ref) { _ in } }
        await eventually("first get") { h.mac.count(AssetGet.type) == 1 }
        await h.advance(until: { h.mac.count(AssetGet.type) == 2 }, "the retry")
        await h.advance(until: { task.isCancelled || h.clock.sleeperCount == 0 }, step: 5, limit: 60, "the final timeout")
        do {
            _ = try await task.value
            XCTFail("expected a timeout")
        } catch {
            XCTAssertEqual(error as? BackendError, .timedOut)
        }
    }

    func testALostWindowIsRecoveredByTheRetry() async throws {
        let h = Harness()
        await h.pair()
        let data = randomData(Wire.assetChunkBytes + 10)
        let ref = asset(size: data.count)
        h.mac.register(asset: ref.assetId, data: data)
        h.mac.dropAssetGets = 1
        let task = Task { try await h.engine.download(macId: h.mac.macId, asset: ref) { _ in } }
        await eventually("first get") { h.mac.count(AssetGet.type) == 1 }
        await h.advance(until: { h.mac.count(AssetGet.type) == 2 }, "the retry")
        let url = try await task.value
        XCTAssertEqual(try Data(contentsOf: url), data)
    }

    func testCancellingTheWaiterFailsItWithoutBreakingTheEngine() async throws {
        let h = Harness()
        await h.pair()
        let ref = asset(size: 100)
        h.mac.register(asset: ref.assetId, data: randomData(100))
        h.mac.dropAssetGets = 1
        let task = Task { try await h.engine.download(macId: h.mac.macId, asset: ref) { _ in } }
        await eventually("get") { h.mac.count(AssetGet.type) == 1 }
        task.cancel()
        do {
            _ = try await task.value
            XCTFail("expected cancellation")
        } catch {
            XCTAssertEqual(error as? BackendError, .cancelled)
        }
        // A later request for the same asset starts fresh and works.
        let url = try await h.engine.download(macId: h.mac.macId, asset: ref) { _ in }
        XCTAssertEqual(try Data(contentsOf: url).count, 100)
    }

    func testTheMacGoingOfflineFailsRunningDownloads() async throws {
        let h = Harness()
        await h.pair()
        let ref = asset(size: 100)
        h.mac.register(asset: ref.assetId, data: randomData(100))
        h.mac.dropAssetGets = 1
        let task = Task { try await h.engine.download(macId: h.mac.macId, asset: ref) { _ in } }
        await eventually("get") { h.mac.count(AssetGet.type) == 1 }
        await h.mac.publishPresence(online: false)
        do {
            _ = try await task.value
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .macOffline)
        }
    }

    // MARK: uploads

    func testAnUploadArrivesIntactInWindowsOfAcknowledgements() async throws {
        let h = Harness()
        let request = await pairedWithRequest(h)
        let data = randomData(Wire.assetChunkBytes * 5 + 77)
        let seen = Counter()
        let id = try await h.engine.upload(
            macId: h.mac.macId, requestId: request.id,
            source: UploadSource(name: "log.txt", mime: "text/plain", data: data, isImage: false)
        ) { seen.record($0) }
        let stored = try XCTUnwrap(h.mac.uploaded(id))
        XCTAssertEqual(stored.name, "log.txt")
        XCTAssertEqual(stored.data, data)
        XCTAssertEqual(h.mac.count(UploadChunk.type), 6)
        XCTAssertEqual(seen.last, 1)
        let begin = try h.mac.received(UploadBegin.type)[0].body(as: UploadBegin.self)
        XCTAssertEqual(begin.chunks, 6)
        XCTAssertEqual(begin.sha256, SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined())
        XCTAssertEqual(begin.scope.requestId, request.id)
    }

    func testASmallUploadIsOneChunk() async throws {
        let h = Harness()
        let request = await pairedWithRequest(h)
        let id = try await h.engine.upload(
            macId: h.mac.macId, requestId: request.id,
            source: UploadSource(name: "a.png", mime: "image/png", data: Data([1, 2, 3]), isImage: true)
        ) { _ in }
        XCTAssertEqual(h.mac.uploaded(id)?.data, Data([1, 2, 3]))
        XCTAssertEqual(h.mac.count(UploadChunk.type), 1)
    }

    func testUploadsAreRefusedLocallyWhenTheyCannotWork() async throws {
        let h = Harness()
        let request = await pairedWithRequest(h)
        let tooBig = UploadSource(name: "big", mime: "x/y", data: Data(count: Wire.maxAssetBytes + 1), isImage: false)
        let empty = UploadSource(name: "empty", mime: "x/y", data: Data(), isImage: false)
        let ok = UploadSource(name: "ok", mime: "x/y", data: Data([1]), isImage: false)
        for (label, source, requestId, expected) in [
            ("too big", tooBig, request.id, BackendError.tooLarge),
            ("empty", empty, request.id, .tooLarge),
            ("unknown request", ok, "nope", .unknownRequest),
        ] {
            do {
                _ = try await h.engine.upload(macId: h.mac.macId, requestId: requestId, source: source) { _ in }
                XCTFail("\(label): expected an error")
            } catch {
                XCTAssertEqual(error as? BackendError, expected, label)
            }
        }
        XCTAssertEqual(h.mac.count(UploadBegin.type), 0)
    }

    func testTheMacRefusingAnUploadSurfacesItsReason() async throws {
        let h = Harness()
        let request = await pairedWithRequest(h)
        h.mac.uploadError = "tooLarge"
        do {
            _ = try await h.engine.upload(
                macId: h.mac.macId, requestId: request.id, source: UploadSource(name: "a", mime: "x/y", data: Data([1]), isImage: false)
            ) { _ in }
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .tooLarge)
        }
        h.mac.uploadError = "gone"
        do {
            _ = try await h.engine.upload(
                macId: h.mac.macId, requestId: request.id, source: UploadSource(name: "a", mime: "x/y", data: Data([1]), isImage: false)
            ) { _ in }
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .unknownRequest)
        }
    }

    func testAnUnansweredUploadTimesOut() async throws {
        let h = Harness()
        let request = await pairedWithRequest(h)
        h.mac.ignoreUploads = true
        let task = Task {
            try await h.engine.upload(
                macId: h.mac.macId, requestId: request.id, source: UploadSource(name: "a", mime: "x/y", data: Data([1]), isImage: false)
            ) { _ in }
        }
        await eventually("begin") { h.mac.count(UploadBegin.type) == 1 }
        await h.advance(until: { h.clock.sleeperCount == 0 }, step: 5, limit: 60, "the acknowledgement timeout")
        do {
            _ = try await task.value
            XCTFail("expected a timeout")
        } catch {
            XCTAssertEqual(error as? BackendError, .timedOut)
        }
    }
}

/// Collects progress callbacks from any thread.
final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var all: [Double] = []

    func record(_ value: Double) { lock.withLock { all.append(value) } }
    var values: [Double] { lock.withLock { all } }
    var last: Double? { values.last }
}
