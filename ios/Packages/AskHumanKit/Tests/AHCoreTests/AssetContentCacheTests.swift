import XCTest
import CryptoKit
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The phone's cache by content (docs/specs/ios-attachment-traffic.md AT4, AT19, §7.1): a request that records the SHA-256
/// of a file lets the phone find it again under a later request's new asset id, the file and its preview each kept once,
/// and a name by content is given only to bytes the phone has seen hash to it. `TestMac` serves only the assets a test
/// registers, so a request the phone makes for one that is not registered fails the test instead of passing quietly.
final class AssetContentCacheTests: XCTestCase {
    private func randomData(_ count: Int) -> Data { Data((0..<count).map { _ in UInt8.random(in: 0...255) }) }

    private func hash(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }

    /// A picture of a request, as a Mac that records hashes describes it. `claiming` is the hash the request says, when it
    /// is not the one of `file` (the file changed after the request was built); `hashed: false` is a Mac, or a picture of
    /// the history, that gives no hash.
    private func picture(_ file: Data, claiming claimed: String? = nil, hashed: Bool = true, preview: Bool = true) -> AssetRef {
        AssetRef(
            assetId: Bytes.newId(), name: "shot.png", size: UInt64(file.count), mime: "image/png", isImage: true,
            preview: preview, sha256: hashed ? (claimed ?? hash(file)) : nil
        )
    }

    private func asks(_ h: Harness) throws -> [AssetGet] {
        try h.mac.received(AssetGet.type).map { try $0.body(as: AssetGet.self) }
    }

    private func fetch(_ h: Harness, _ ref: AssetRef) async throws -> URL {
        let engine = h.engine, macId = h.mac.macId
        return try await finish(Task { try await engine.download(macId: macId, asset: ref) { _ in } })
    }

    private func view(_ h: Harness, _ ref: AssetRef) async throws -> ViewableAsset {
        let engine = h.engine, macId = h.mac.macId
        return try await finish(Task { try await engine.downloadForViewing(macId: macId, asset: ref) { _ in } })
    }

    private func assetDirectory(_ h: Harness) -> URL { h.root.appendingPathComponent("assets") }

    private func names(in directory: URL) -> [String] {
        ((try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []).sorted()
    }

    /// How many names the file at `url` has (a file kept under its asset id and under its hash has two).
    private func links(to url: URL) throws -> Int {
        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        return try XCTUnwrap((attributes[.referenceCount] as? NSNumber)?.intValue)
    }

    private func exists(_ url: URL) -> Bool { FileManager.default.fileExists(atPath: url.path) }

    // MARK: the file

    func testTheSameContentInAnotherRequestIsNotDownloadedAgain() async throws {
        let h = Harness()
        await h.pair()
        let file = randomData(Wire.assetChunkBytes * 3 + 77)
        let first = picture(file, preview: false)
        let second = picture(file, preview: false)
        h.mac.register(asset: first.assetId, data: file)
        // The Mac has nothing under the second asset id: a request for it would be answered `notFound`.

        let url = try await fetch(h, first)
        XCTAssertEqual(try Data(contentsOf: url), file)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1)

        let again = try await fetch(h, second)
        XCTAssertEqual(try Data(contentsOf: again), file)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1, "the second request is served from what the first one brought")
        let kept = await h.engine.cachedContentURL(hash(file))
        XCTAssertEqual(again, kept)
        XCTAssertEqual(try links(to: kept), 2, "one file with two names, not two copies")
    }

    func testTheAssetIdIsLookedUpBeforeTheContent() async throws {
        let h = Harness()
        await h.pair()
        // Two files of one size; the one under the asset id is not the one under the hash, so the answer says which was used.
        let byId = randomData(900), byContent = randomData(900)
        let first = picture(byContent, preview: false)
        let other = picture(byContent, preview: false)
        let directory = assetDirectory(h)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let ownName = await h.engine.cachedAssetURL(first.assetId)
        let contentName = await h.engine.cachedContentURL(hash(byContent))
        try byId.write(to: ownName)
        try byContent.write(to: contentName)

        let own = try await fetch(h, first)
        XCTAssertEqual(try Data(contentsOf: own), byId)
        let shared = try await fetch(h, other)
        XCTAssertEqual(try Data(contentsOf: shared), byContent)
        XCTAssertEqual(h.mac.count(AssetGet.type), 0)
    }

    func testAFileThatIsNotWhatTheRequestSaidIsShownButNotKeptByContent() async throws {
        let h = Harness()
        await h.pair()
        // The request was built from `said`; by the time the phone asks, the file on the Mac is `now` (AT19).
        let said = randomData(5_000), now = randomData(5_000)
        let changed = picture(now, claiming: hash(said), preview: false)
        let honest = picture(said, preview: false)
        h.mac.register(asset: changed.assetId, data: now)
        h.mac.register(asset: honest.assetId, data: said)

        let shown = try await fetch(h, changed)
        XCTAssertEqual(try Data(contentsOf: shown), now, "it is shown as usual")
        let name = await h.engine.cachedContentURL(hash(said))
        XCTAssertFalse(exists(name), "the hash says `said`, and these bytes are not")
        let own = await h.engine.cachedAssetURL(changed.assetId)
        XCTAssertEqual(shown, own, "it stays under its asset id")

        // So a request that really is for `said` is not answered with them.
        let real = try await fetch(h, honest)
        XCTAssertEqual(try Data(contentsOf: real), said)
        XCTAssertEqual(h.mac.count(AssetGet.type), 2)
        XCTAssertTrue(exists(name), "these bytes are `said`: now they are kept by it")
        // And what is under the asset id keeps serving that asset.
        _ = try await fetch(h, changed)
        XCTAssertEqual(h.mac.count(AssetGet.type), 2)
    }

    func testAFileOfTheWrongSizeUnderAHashIsNotAHitAndIsReplaced() async throws {
        let h = Harness()
        await h.pair()
        let file = randomData(Wire.assetChunkBytes + 500)
        let ref = picture(file, preview: false)
        h.mac.register(asset: ref.assetId, data: file)
        let name = await h.engine.cachedContentURL(hash(file))
        try FileManager.default.createDirectory(at: assetDirectory(h), withIntermediateDirectories: true)
        try file.prefix(100).write(to: name)

        _ = try await fetch(h, ref)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1, "a truncated file is not the content")
        XCTAssertEqual(try Data(contentsOf: name), file, "what comes in its place is the whole file")
    }

    func testTheContentCacheServesAMacThatIsAway() async throws {
        let h = Harness()
        await h.pair()
        let file = randomData(4_000)
        let first = picture(file, preview: false), second = picture(file, preview: false)
        h.mac.register(asset: first.assetId, data: file)
        _ = try await fetch(h, first)

        await h.mac.publishPresence(online: false)
        await eventually("offline") { await h.state().macs.first?.link == .macOffline }
        let url = try await fetch(h, second)
        XCTAssertEqual(try Data(contentsOf: url), file)

        let engine = h.engine, macId = h.mac.macId
        let exported = try await finish(Task { try await engine.downloadForExport(macId: macId, asset: second) { _ in } })
        XCTAssertEqual(try Data(contentsOf: exported), file, "the export path finds it too, without waiting for the link")
        XCTAssertEqual(h.mac.count(AssetGet.type), 1)
    }

    // MARK: the preview

    func testAPreviewIsKeptByTheHashOfTheFileItWasMadeFrom() async throws {
        let h = Harness()
        await h.pair()
        let file = randomData(Wire.assetChunkBytes * 2 + 9)
        let preview = randomData(30_000)
        let first = picture(file), second = picture(file)
        h.mac.register(asset: first.assetId, data: file)
        h.mac.register(preview: first.assetId, data: preview)

        let viewed = try await view(h, first)
        XCTAssertTrue(viewed.isPreview)
        XCTAssertEqual(try asks(h).map(\.variant), ["preview"])

        let again = try await view(h, second)
        XCTAssertTrue(again.isPreview)
        XCTAssertEqual(try Data(contentsOf: again.url), preview)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1, "the preview of the same file is not fetched twice")
        let kept = await h.engine.cachedContentPreviewURL(hash(file))
        XCTAssertEqual(again.url, kept)
        XCTAssertEqual(try links(to: kept), 2)
        let fileName = await h.engine.cachedContentURL(hash(file))
        XCTAssertFalse(exists(fileName), "a preview is not the file, so nothing is kept under the file's name")
    }

    func testTheFileThatIsHereByContentIsShownInPlaceOfAPreview() async throws {
        let h = Harness()
        await h.pair()
        let file = randomData(Wire.assetChunkBytes + 5)
        let first = picture(file, preview: false), second = picture(file)
        h.mac.register(asset: first.assetId, data: file)
        _ = try await fetch(h, first)

        let viewed = try await view(h, second)
        XCTAssertFalse(viewed.isPreview)
        XCTAssertEqual(try Data(contentsOf: viewed.url), file)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1)
    }

    func testAPreviewThatCameAsTheFileIsKeptAsTheFile() async throws {
        let h = Harness()
        await h.pair()
        let file = randomData(Wire.assetChunkBytes * 2 + 1)
        let first = picture(file), second = picture(file)
        h.mac.register(asset: first.assetId, data: file)
        h.mac.answerPreviewsWithFiles = true

        let viewed = try await view(h, first)
        XCTAssertFalse(viewed.isPreview)
        let fileName = await h.engine.cachedContentURL(hash(file))
        let previewName = await h.engine.cachedContentPreviewURL(hash(file))
        XCTAssertTrue(exists(fileName), "the phone checked these bytes against the hash")
        XCTAssertFalse(exists(previewName))

        let again = try await view(h, second)
        XCTAssertEqual(try Data(contentsOf: again.url), file)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1)
    }

    func testAFileThatChangedIsNotKeptEvenWhenItComesInPlaceOfAPreview() async throws {
        let h = Harness()
        await h.pair()
        let said = randomData(6_000), now = randomData(6_000)
        let first = picture(now, claiming: hash(said))
        h.mac.register(asset: first.assetId, data: now)
        h.mac.answerPreviewsWithFiles = true

        let viewed = try await view(h, first)
        XCTAssertFalse(viewed.isPreview)
        XCTAssertEqual(try Data(contentsOf: viewed.url), now)
        XCTAssertEqual(names(in: assetDirectory(h)), [first.assetId], "nothing is named after the hash")
    }

    // MARK: what is never used

    func testARequestWithoutAHashChangesNothing() async throws {
        let h = Harness()
        await h.pair()
        // An older Mac, or a picture of the history (AT13): the same content under two asset ids is two downloads.
        let file = randomData(Wire.assetChunkBytes + 40)
        let preview = randomData(10_000)
        let first = picture(file, hashed: false), second = picture(file, hashed: false)
        for ref in [first, second] {
            h.mac.register(asset: ref.assetId, data: file)
            h.mac.register(preview: ref.assetId, data: preview)
        }

        _ = try await view(h, first)
        _ = try await view(h, second)
        _ = try await fetch(h, first)
        _ = try await fetch(h, second)
        XCTAssertEqual(try asks(h).map(\.variant), ["preview", "preview", nil, nil])
        let expected = [first.assetId, first.assetId + ".preview", second.assetId, second.assetId + ".preview"].sorted()
        XCTAssertEqual(names(in: assetDirectory(h)), expected, "only the names it always had")
    }

    func testAHashThatIsNotAHashIsNeverMadeIntoAFileName() async throws {
        let h = Harness()
        await h.pair()
        let file = randomData(3_000)
        // A file of the right size where a path in a hash would lead: a phone that took the hash for a name would find it
        // and show it. It sits next to the asset directory, which is where `../escaped` goes.
        let planted = randomData(file.count)
        let outside = h.root.appendingPathComponent("escaped")
        try planted.write(to: outside)
        for claim in ["../escaped", String(hash(file).dropLast()), hash(file) + "0", "zz" + String(hash(file).dropFirst(2)), ""] {
            // The engine paces what it publishes on the manual clock (a burst of ten); a few downloads in a row would
            // otherwise wait for a clock that nothing moves.
            await h.clock.advance(by: 10)
            let first = picture(file, claiming: claim, preview: false)
            let second = picture(file, claiming: claim, preview: false)
            h.mac.register(asset: first.assetId, data: file)
            h.mac.register(asset: second.assetId, data: file)
            let before = h.mac.count(AssetGet.type)
            let shown = try await fetch(h, first)
            XCTAssertEqual(try Data(contentsOf: shown), file, "\(claim) leads nowhere")
            _ = try await fetch(h, second)
            XCTAssertEqual(h.mac.count(AssetGet.type), before + 2, "\(claim) is no key: both are downloaded")
        }
        let longNames = names(in: assetDirectory(h)).filter { $0.utf8.count > 26 }
        XCTAssertEqual(longNames, [], "nothing but asset ids became files")
        XCTAssertEqual(try Data(contentsOf: outside), planted, "nothing was written where a path in a hash points")
    }

    func testTheCaseOfAHashIsNotPartOfWhatItSays() async throws {
        let h = Harness()
        await h.pair()
        let file = randomData(2_000)
        let shouted = picture(file, claiming: hash(file).uppercased(), preview: false)
        let quiet = picture(file, preview: false)
        h.mac.register(asset: shouted.assetId, data: file)

        _ = try await fetch(h, shouted)
        let url = try await fetch(h, quiet)
        XCTAssertEqual(try Data(contentsOf: url), file)
        XCTAssertEqual(h.mac.count(AssetGet.type), 1)
        XCTAssertTrue(names(in: assetDirectory(h)).contains(hash(file)), "named in lowercase, as the Mac writes it")
    }
}
