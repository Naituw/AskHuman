import XCTest
import CryptoKit
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The phone's side of the previews (docs/specs/ios-attachment-traffic.md §7.1): `downloadForViewing` asks a Mac that
/// declares `assetPreview` for the preview of a picture that offers one, keeps preview and file apart in the cache and in
/// the downloads under way, and treats the file the Mac sends in place of a preview as the file. `download` and the export
/// path are the file always. `TestMac` is the Mac of the real build (`assetPreview` on) unless a test turns that off.
final class AssetPreviewTests: XCTestCase {
    private func randomData(_ count: Int) -> Data { Data((0..<count).map { _ in UInt8.random(in: 0...255) }) }

    /// A picture the Mac describes as one that has a preview (a small stand-in for a screenshot of a few megabytes).
    private func picture(size: Int, preview: Bool = true) -> AssetRef {
        AssetRef(assetId: Bytes.newId(), name: "shot.png", size: UInt64(size), mime: "image/png", isImage: true, preview: preview)
    }

    private func asks(_ h: Harness) throws -> [AssetGet] {
        try h.mac.received(AssetGet.type).map { try $0.body(as: AssetGet.self) }
    }

    /// Several windows of the file, one of the preview: what a big screenshot and its preview look like to the phone.
    private func fixture(_ h: Harness, fileChunks: Int = 4, previewBytes: Int = 20_000) -> (ref: AssetRef, file: Data, preview: Data) {
        let file = randomData(Wire.assetChunkBytes * fileChunks - 321)
        let preview = randomData(previewBytes)
        let ref = picture(size: file.count)
        h.mac.register(asset: ref.assetId, data: file)
        h.mac.register(preview: ref.assetId, data: preview)
        return (ref, file, preview)
    }

    private func viewing(_ h: Harness, _ ref: AssetRef, progress: @escaping @Sendable (Double) -> Void = { _ in }) -> Task<ViewableAsset, Error> {
        let engine = h.engine, macId = h.mac.macId
        return Task { try await engine.downloadForViewing(macId: macId, asset: ref, progress: progress) }
    }

    private func fetching(_ h: Harness, _ ref: AssetRef) -> Task<URL, Error> {
        let engine = h.engine, macId = h.mac.macId
        return Task { try await engine.download(macId: macId, asset: ref) { _ in } }
    }

    private func view(_ h: Harness, _ ref: AssetRef, progress: @escaping @Sendable (Double) -> Void = { _ in }) async throws -> ViewableAsset {
        try await finish(viewing(h, ref, progress: progress))
    }

    private func fetch(_ h: Harness, _ ref: AssetRef) async throws -> URL {
        try await finish(fetching(h, ref))
    }

    // MARK: what is asked for

    func testViewingAPictureFetchesItsPreviewAndKeepsItApartFromTheFile() async throws {
        let h = Harness()
        await h.pair()
        let (ref, _, preview) = fixture(h)

        let seen = Counter()
        let viewed = try await view(h, ref) { seen.record($0) }
        XCTAssertTrue(viewed.isPreview)
        XCTAssertEqual(try Data(contentsOf: viewed.url), preview)
        XCTAssertEqual(seen.last, 1)
        XCTAssertEqual(try asks(h).map(\.variant), ["preview"])

        let previewURL = await h.engine.cachedPreviewURL(ref.assetId)
        let fileURL = await h.engine.cachedAssetURL(ref.assetId)
        XCTAssertEqual(viewed.url, previewURL)
        XCTAssertNotEqual(previewURL, fileURL)
        XCTAssertFalse(FileManager.default.fileExists(atPath: fileURL.path), "no file was fetched")

        let again = try await view(h, ref)
        XCTAssertEqual(again, viewed)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1, "the second look is served from the cache")
    }

    func testDownloadStillBringsTheFileAndViewingThenShowsIt() async throws {
        let h = Harness()
        await h.pair()
        let (ref, file, _) = fixture(h)

        let viewed = try await view(h, ref)
        XCTAssertTrue(viewed.isPreview)
        let url = try await fetch(h, ref)
        XCTAssertEqual(try Data(contentsOf: url), file, "a preview on the phone does not stand in for the file")
        XCTAssertEqual(try asks(h).map(\.variant), ["preview", nil])

        let afterwards = try await view(h, ref)
        XCTAssertEqual(afterwards, ViewableAsset(url: url, isPreview: false), "the file is what a viewer shows once it is here")
        XCTAssertEqual(h.mac.count(AssetGet.type), 2)
    }

    func testTheExportPathIsTheFileEvenWhenAPreviewIsCached() async throws {
        let h = Harness()
        await h.pair()
        let (ref, file, _) = fixture(h)
        _ = try await view(h, ref)

        let engine = h.engine, macId = h.mac.macId
        let url = try await finish(Task { try await engine.downloadForExport(macId: macId, asset: ref) { _ in } })
        XCTAssertEqual(try Data(contentsOf: url), file)
        XCTAssertEqual(try asks(h).map(\.variant), ["preview", nil])
    }

    func testAMacThatDoesNotMakePreviewsIsNotAskedForOne() async throws {
        let h = Harness()
        h.mac.caps.assetPreview = false
        await h.pair()
        let (ref, file, _) = fixture(h)

        let viewed = try await view(h, ref)
        XCTAssertFalse(viewed.isPreview)
        XCTAssertEqual(try Data(contentsOf: viewed.url), file)
        XCTAssertEqual(try asks(h).map(\.variant), [nil], "an older Mac would take the field for nothing and send the file")
    }

    func testAPictureWithoutAPreviewIsFetchedAsTheFile() async throws {
        let h = Harness()
        await h.pair()
        let file = randomData(5_000)
        let ref = picture(size: file.count, preview: false)
        h.mac.register(asset: ref.assetId, data: file)
        h.mac.register(preview: ref.assetId, data: randomData(100))

        let viewed = try await view(h, ref)
        XCTAssertFalse(viewed.isPreview)
        XCTAssertEqual(try Data(contentsOf: viewed.url), file)
        XCTAssertEqual(try asks(h).map(\.variant), [nil])
    }

    func testAPreviewThatIsOnThePhoneIsShownWhateverTheLinkIsDoing() async throws {
        let h = Harness()
        await h.pair()
        let (ref, _, preview) = fixture(h)
        _ = try await view(h, ref)

        await h.mac.publishPresence(online: false)
        await eventually("offline") { await h.state().macs.first?.link == .macOffline }
        let viewed = try await view(h, ref)
        XCTAssertEqual(try Data(contentsOf: viewed.url), preview)

        do {
            _ = try await view(h, picture(size: 1_000))
            XCTFail("a picture that is not here cannot come from a Mac that is away")
        } catch {
            XCTAssertEqual(error as? BackendError, .macOffline)
        }
    }

    // MARK: preview and file together

    func testThePreviewArrivesWhileTheFileOfTheSameAssetIsStillOnItsWay() async throws {
        let h = Harness()
        await h.pair()
        let (ref, file, preview) = fixture(h, fileChunks: 19)
        // The first request for the file is lost, so the file is certain to be under way when the viewer asks.
        h.mac.dropAssetGets = 1
        let toFile = fetching(h, ref)
        await eventually("the file is asked for") { h.mac.count(AssetGet.type) == 1 }

        let viewed = try await view(h, ref)
        XCTAssertTrue(viewed.isPreview)
        XCTAssertEqual(try Data(contentsOf: viewed.url), preview)
        let underWay = await h.engine.downloads.keys.map(\.variant)
        XCTAssertEqual(underWay, [.original], "only the file is left")

        await h.advance(until: { await h.engine.downloads.isEmpty }, "the file arrives")
        let fileURL = try await finish(toFile)
        XCTAssertEqual(try Data(contentsOf: fileURL), file)
        XCTAssertEqual(try Data(contentsOf: viewed.url), preview, "the preview stays what it was")
        XCTAssertEqual(Array(try asks(h).map(\.variant).prefix(2)), [nil, "preview"])
    }

    func testThePreviewAndTheFileOfOneAssetCanBeOnTheirWayAtTheSameTime() async throws {
        let h = Harness()
        await h.pair()
        // 19 chunks are three windows; the preview is three chunks of its own.
        let (ref, file, preview) = fixture(h, fileChunks: 19, previewBytes: Wire.assetChunkBytes * 3 - 10)

        let toFile = fetching(h, ref)
        let toViewer = viewing(h, ref)
        let fileURL = try await finish(toFile)
        let viewed = try await finish(toViewer)

        XCTAssertEqual(try Data(contentsOf: fileURL), file)
        // Whether the viewer got the preview depends on which finished first: a file that is on the phone by then is
        // what it is given (§7.1). Either way what it is given is what it says it is.
        XCTAssertEqual(try Data(contentsOf: viewed.url), viewed.isPreview ? preview : file)
        let previewURL = await h.engine.cachedPreviewURL(ref.assetId)
        XCTAssertEqual(viewed.url, viewed.isPreview ? previewURL : fileURL)
        let leftOver = await h.engine.downloads.count
        XCTAssertEqual(leftOver, 0)
    }

    func testTwoViewersOfOnePictureShareOneDownload() async throws {
        let h = Harness()
        await h.pair()
        let (ref, _, preview) = fixture(h)

        let first = viewing(h, ref)
        let second = viewing(h, ref)
        let a = try await finish(first)
        let b = try await finish(second)

        XCTAssertEqual(a, b)
        XCTAssertEqual(try Data(contentsOf: a.url), preview)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1)
    }

    // MARK: the Mac sends the file

    func testAMacThatSendsTheFileInPlaceOfAPreviewHandsOverTheFile() async throws {
        let h = Harness()
        await h.pair()
        // Ten chunks: the first window is answered while the download is still "a preview", the second must not be.
        let (ref, file, _) = fixture(h, fileChunks: 10)
        h.mac.answerPreviewsWithFiles = true

        let seen = Counter()
        let viewed = try await view(h, ref) { seen.record($0) }
        XCTAssertFalse(viewed.isPreview)
        XCTAssertEqual(try Data(contentsOf: viewed.url), file)
        let fileURL = await h.engine.cachedAssetURL(ref.assetId)
        let previewURL = await h.engine.cachedPreviewURL(ref.assetId)
        XCTAssertEqual(viewed.url, fileURL)
        XCTAssertFalse(FileManager.default.fileExists(atPath: previewURL.path))
        XCTAssertEqual(seen.values, seen.values.sorted(), "progress never goes backwards")
        XCTAssertEqual(seen.last, 1)

        let gets = try asks(h)
        XCTAssertEqual(gets.map(\.from), [0, 8])
        XCTAssertEqual(gets.map(\.variant), ["preview", nil], "once it is the file, the rest is asked for as the file")

        let again = try await view(h, ref)
        XCTAssertEqual(again, viewed)
        XCTAssertEqual(h.mac.count(AssetGet.type), 2)
        let leftOver = await h.engine.downloads.count
        XCTAssertEqual(leftOver, 0)
    }

    func testAFileAlreadyOnItsWayTakesInAPreviewRequestTheMacAnswersWithTheFile() async throws {
        let h = Harness()
        await h.pair()
        let (ref, file, _) = fixture(h, fileChunks: 3)
        h.mac.answerPreviewsWithFiles = true
        h.mac.dropAssetGets = 1

        let toFile = fetching(h, ref)
        await eventually("the file is asked for") { h.mac.count(AssetGet.type) == 1 }
        let toViewer = viewing(h, ref)

        let fileURL = try await finish(toFile)
        let viewed = try await finish(toViewer)
        XCTAssertEqual(try Data(contentsOf: fileURL), file)
        XCTAssertEqual(viewed, ViewableAsset(url: fileURL, isPreview: false))
        XCTAssertEqual(try asks(h).map(\.variant), [nil, "preview"], "the one answer was enough for both")
        let leftOver = await h.engine.downloads.count
        XCTAssertEqual(leftOver, 0)
    }

    func testAFileThatArrivesWhileAViewerWaitsForAPreviewIsWhatTheViewerGets() async throws {
        let h = Harness()
        await h.pair()
        let (ref, file, _) = fixture(h, fileChunks: 3)
        h.mac.dropAssetGets = 1

        let toViewer = viewing(h, ref)
        await eventually("the preview is asked for") { h.mac.count(AssetGet.type) == 1 }
        let fileURL = try await fetch(h, ref)

        let viewed = try await finish(toViewer)
        XCTAssertEqual(viewed, ViewableAsset(url: fileURL, isPreview: false))
        XCTAssertEqual(try Data(contentsOf: viewed.url), file)
        let leftOver = await h.engine.downloads.count
        XCTAssertEqual(leftOver, 0, "the preview request that is still out is not waited for")
    }

    // MARK: failures

    func testAChecksumMismatchOfAPreviewFailsAndStoresNothing() async throws {
        let h = Harness()
        await h.pair()
        let (ref, _, _) = fixture(h)
        h.mac.corruptAssetChecksum = true
        do {
            _ = try await view(h, ref)
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .invalid("Checksum mismatch."))
        }
        let path = await h.engine.cachedPreviewURL(ref.assetId).path
        XCTAssertFalse(FileManager.default.fileExists(atPath: path))
    }

    func testAnErrorOfTheMacFailsTheViewer() async throws {
        let h = Harness()
        await h.pair()
        do {
            _ = try await view(h, picture(size: 1_000))
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .unavailable("notFound"))
        }
        let leftOver = await h.engine.downloads.count
        XCTAssertEqual(leftOver, 0)
    }

    func testAPictureMarkedTooLargeIsNotRequestedForViewing() async throws {
        let h = Harness()
        await h.pair()
        var ref = picture(size: 20_000_000)
        ref.tooLarge = true
        do {
            _ = try await view(h, ref)
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .tooLarge)
        }
        XCTAssertEqual(h.mac.count(AssetGet.type), 0)
    }

    func testALostWindowOfAPreviewIsAskedForAgainAsAPreview() async throws {
        let h = Harness()
        await h.pair()
        let (ref, _, preview) = fixture(h)
        h.mac.dropAssetGets = 1

        let task = viewing(h, ref)
        await eventually("the preview is asked for") { h.mac.count(AssetGet.type) == 1 }
        await h.advance(until: { h.mac.count(AssetGet.type) == 2 }, "the preview is asked for again")

        let viewed = try await finish(task)
        XCTAssertTrue(viewed.isPreview)
        XCTAssertEqual(try Data(contentsOf: viewed.url), preview)
        XCTAssertEqual(try asks(h).map(\.variant), ["preview", "preview"])
    }

    func testCancellingTheViewerDropsItsDownloadAndALaterLookStartsAgain() async throws {
        let h = Harness()
        await h.pair()
        let (ref, _, preview) = fixture(h)
        h.mac.dropAssetGets = 1

        let task = viewing(h, ref)
        await eventually("the preview is asked for") { h.mac.count(AssetGet.type) == 1 }
        task.cancel()
        let outcome = await task.result
        guard case .failure(let error) = outcome else { return XCTFail("a cancelled viewer does not get a picture") }
        XCTAssertEqual(error as? BackendError, .cancelled)
        await eventually("the download is gone") { await h.engine.downloads.isEmpty }

        let viewed = try await view(h, ref)
        XCTAssertEqual(try Data(contentsOf: viewed.url), preview)
        XCTAssertEqual(h.mac.count(AssetGet.type), 2)
    }

    // MARK: chunks the phone does not expect

    /// A chunk as the Mac would send it, handed to the engine directly.
    private func chunk(_ ref: AssetRef, index: UInt64 = 0, total: UInt64, size: UInt64, variant: String?, bytes: Data) -> AssetChunk {
        AssetChunk(assetId: ref.assetId, index: index, total: total, size: size, sha256: nil, variant: variant, data: Bytes.b64u(bytes))
    }

    func testAFileThatFollowsChunksOfAPreviewFailsInsteadOfMixingTheTwo() async throws {
        let h = Harness()
        await h.pair()
        let ref = picture(size: 5_000_000)
        h.mac.dropAssetGets = 1
        let task = viewing(h, ref)
        await eventually("the preview is asked for") { h.mac.count(AssetGet.type) == 1 }

        let bytes = randomData(1_000)
        await h.engine.onAssetChunk(
            h.mac.macId, chunk(ref, total: 9, size: 1_000_000, variant: "preview", bytes: bytes), blob: nil
        )
        await h.engine.onAssetChunk(
            h.mac.macId, chunk(ref, index: 1, total: 26, size: 5_000_000, variant: "original", bytes: bytes), blob: nil
        )
        do {
            _ = try await finish(task)
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .invalid("The file changed."))
        }
        let leftOver = await h.engine.downloads.count
        XCTAssertEqual(leftOver, 0)
    }

    func testAChunkOfAVariantThePhoneNeverAskedForIsIgnored() async throws {
        let h = Harness()
        await h.pair()
        let (ref, file, preview) = fixture(h, fileChunks: 3)
        // Both downloads of the asset are waiting, so a chunk that is taken for either of them would be seen.
        h.mac.dropAssetGets = 2
        let toViewer = viewing(h, ref)
        await eventually("the preview is asked for") { h.mac.count(AssetGet.type) == 1 }
        let toFile = fetching(h, ref)
        await eventually("the file is asked for") { h.mac.count(AssetGet.type) == 2 }

        // As small as the file is not, so that it would be "complete" in either of them.
        await h.engine.onAssetChunk(
            h.mac.macId, chunk(ref, total: 1, size: 100, variant: "sharpened", bytes: randomData(100)), blob: nil
        )
        let waiting = await h.engine.downloads.keys.map(\.variant).sorted { $0.rawValue < $1.rawValue }
        XCTAssertEqual(waiting, [.original, .preview], "the chunk neither completed nor failed a download")
        let path = await h.engine.cachedAssetURL(ref.assetId).path
        XCTAssertFalse(FileManager.default.fileExists(atPath: path))

        await h.advance(until: { await h.engine.downloads.isEmpty }, "both are asked for again")
        let fileURL = try await finish(toFile)
        XCTAssertEqual(try Data(contentsOf: fileURL), file)
        // Which of the two the viewer gets depends on which answer came first; either is what it says it is.
        let viewed = try await finish(toViewer)
        XCTAssertEqual(try Data(contentsOf: viewed.url), viewed.isPreview ? preview : file)
    }

    func testChunksOfAFileNobodyWaitsForAreDropped() async throws {
        let h = Harness()
        await h.pair()
        let ref = picture(size: 100)
        // The answer to a preview request that was cancelled: nothing is waiting, nothing is kept.
        await h.engine.onAssetChunk(
            h.mac.macId, chunk(ref, total: 1, size: 100, variant: "original", bytes: randomData(100)), blob: nil
        )
        await h.engine.onAssetChunk(
            h.mac.macId, chunk(ref, total: 1, size: 100, variant: "preview", bytes: randomData(100)), blob: nil
        )
        let leftOver = await h.engine.downloads.count
        XCTAssertEqual(leftOver, 0)
        let path = await h.engine.cachedPreviewURL(ref.assetId).path
        XCTAssertFalse(FileManager.default.fileExists(atPath: path))
    }

    // MARK: the app's way in

    /// The app holds the engine as a `RemoteBackend`, and that protocol gives `downloadForViewing` a default that returns the
    /// file. An engine method that only resembles the requirement (a parameter that differs in a way the compiler accepts)
    /// would leave the default in charge: every viewer in the app would show files, no preview would ever be asked for, and no
    /// test that calls the engine itself would notice. This one calls through the protocol, as `AppStore` does.
    func testTheAppReachesThePreviewsThroughTheBackendProtocol() async throws {
        let h = Harness()
        await h.pair()
        let (ref, _, preview) = fixture(h)
        let backend: any RemoteBackend = h.engine
        let macId = h.mac.macId

        let viewed = try await finish(Task { try await backend.downloadForViewing(macId: macId, asset: ref) { _ in } })
        XCTAssertTrue(viewed.isPreview, "the viewer got the file: the protocol's default answered, not the engine")
        XCTAssertEqual(try Data(contentsOf: viewed.url), preview)
        XCTAssertEqual(try asks(h).map(\.variant), ["preview"])

        // The default would have fetched the file for the second look as well.
        let again = try await finish(Task { try await backend.downloadForViewing(macId: macId, asset: ref) { _ in } })
        XCTAssertEqual(again, viewed)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1, "the second look is served from the cache")
    }
}
