import XCTest
import ImageIO
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The pictures in the history (docs/specs/ios-history-pictures.md): the Swift phone engine against the real Rust gateway in
/// a real daemon, reading the throwaway Mac's own `history.jsonl` and the real image files it points at. Every exchange and
/// picture here is made up.
///
///   AH_E2E_BIN=<installed AskHuman> AH_E2E_BROKER_DIR=ios/Tools/local-broker swift test --filter HistoryPicturesE2ETests
final class HistoryPicturesE2ETests: E2ECase {
    private let session = "e2e-pictures-session"

    // MARK: helpers

    private func macId() async throws -> String { try unwrap(await phone.state().macs.first?.id) }

    private var folder: URL { mac.home.appendingPathComponent("pictures", isDirectory: true) }

    /// A real PNG. Noise does not compress, so a noisy one is several chunks of a download and a quiet one is a single chunk.
    private func png(width: Int, height: Int, noise: Bool) throws -> Data {
        var pixels = [UInt8](repeating: 255, count: width * height * 4)
        var generator = SystemRandomNumberGenerator()
        for y in 0..<height {
            for x in 0..<width {
                let at = (y * width + x) * 4
                if noise {
                    pixels[at] = .random(in: 0...255, using: &generator)
                    pixels[at + 1] = .random(in: 0...255, using: &generator)
                    pixels[at + 2] = .random(in: 0...255, using: &generator)
                } else {
                    pixels[at] = UInt8(x * 255 / max(width - 1, 1))
                    pixels[at + 1] = UInt8(y * 255 / max(height - 1, 1))
                    pixels[at + 2] = 128
                }
            }
        }
        guard let provider = CGDataProvider(data: Data(pixels) as CFData),
              let image = CGImage(
                  width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
                  space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
                  provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent
              )
        else { throw XCTSkip("could not draw a picture") }
        let data = NSMutableData()
        let destination = try unwrap(CGImageDestinationCreateWithData(data, "public.png" as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, nil)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return data as Data
    }

    @discardableResult
    private func write(_ data: Data, as name: String) throws -> URL {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appendingPathComponent(name)
        try data.write(to: url)
        return url
    }

    /// One recorded reply, as a line of `history.jsonl`: `files` are the message's (path, name, size, isImage) and `replyImages`
    /// the paths of the pictures pasted into the answer.
    private func entry(
        _ id: String, at ms: Int64, files: [(url: URL, size: Int, isImage: Bool)] = [], replyImages: [URL] = []
    ) throws -> String {
        let object: [String: Any] = [
            "id": id, "timestampMs": ms, "project": "/made/up/project", "source": "E2E", "channel": "popup", "action": "send",
            "agentKind": "claude", "agentSessionId": session, "isMarkdown": false,
            "message": [
                "text": "Message of \(id)",
                "files": files.map { ["path": $0.url.path, "name": $0.url.lastPathComponent, "size": $0.size, "isImage": $0.isImage] },
            ],
            "questions": [["message": "Question of \(id)?", "predefinedOptions": [["text": "Yes", "recommended": false]]]],
            "answers": [["selectedOptions": ["Yes"], "images": replyImages.map(\.path), "files": []]],
        ]
        return String(decoding: try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]), as: UTF8.self)
    }

    private func writeHistory(_ lines: [String]) throws {
        try (lines.joined(separator: "\n") + "\n")
            .write(to: mac.configDir.appendingPathComponent("history.jsonl"), atomically: true, encoding: .utf8)
    }

    private func page() async throws -> SessionHistory {
        try await phone.engine.sessionHistory(
            macId: try await macId(), agentKey: "claude:\(session)", beforeMs: Int64.max, beforeId: nil, limit: Wire.sessionHistoryPage
        )
    }

    private struct Prepared {
        var original: Data
        var reply: Data
        var exchangeId: String
    }

    /// An exchange with a noisy picture (several chunks), one whose file is gone, a text file, and a picture in the answer.
    private func prepare() async throws -> Prepared {
        let original = try png(width: 600, height: 500, noise: true)
        let reply = try png(width: 160, height: 120, noise: false)
        let a = try write(original, as: "screen-a.png")
        let replyFile = try write(reply, as: "reply.png")
        let notes = try write(Data("notes".utf8), as: "notes.txt")
        let gone = folder.appendingPathComponent("cleaned-up.png")
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        try writeHistory([
            try entry(
                "with-pictures", at: now - 60_000,
                files: [(a, original.count, true), (gone, 4_096, true), (notes, 5, false)], replyImages: [replyFile]
            ),
        ])
        return Prepared(original: original, reply: reply, exchangeId: "with-pictures")
    }

    // MARK: the page

    func testThePicturesOfAPageAreDescribedWithoutAnyPathAndTheOriginalsDownloadIntact() async throws {
        try await pair()
        let made = try await prepare()
        let caps = try unwrap(await phone.state().macs.first?.caps)
        XCTAssertTrue(caps.historyPictures, "the installed Mac describes pictures")

        let result = try await page()
        let item = try unwrap(result.items.first)
        XCTAssertEqual(item.id, made.exchangeId)
        XCTAssertEqual(item.attachments, 3, "the count still has the text file")
        XCTAssertEqual(item.pictures.map(\.name), ["screen-a.png", "cleaned-up.png"], "the images, in the order they were attached")
        XCTAssertEqual(item.pictures.map(\.gone), [false, true])
        XCTAssertEqual(item.pictures[0].size, UInt64(made.original.count))
        let thumb = try unwrap(item.pictures[0].thumb.flatMap(Bytes.b64uDecode))
        XCTAssertEqual(Array(thumb.prefix(2)), [0xFF, 0xD8], "a JPEG")
        XCTAssertLessThanOrEqual(thumb.count, Wire.historyThumbMaxBytes)
        XCTAssertNil(item.pictures[1].thumb, "a file that is gone has no thumbnail")
        let reply = try unwrap(item.qs.first)
        XCTAssertEqual(reply.images, 1)
        XCTAssertEqual(reply.pictures.map(\.name), ["reply.png"])

        // The wire never carried a path.
        let json = String(decoding: try AHJSON.encoder.encode(result), as: UTF8.self)
        XCTAssertFalse(json.contains(mac.home.path), "a path travelled")
        XCTAssertFalse(json.contains("/made/up/project"), "a project path travelled")

        // A noisy picture is several chunks; it arrives byte for byte.
        let seen = ProgressLog()
        let id = try await macId()
        let url = try await phone.engine.download(macId: id, asset: item.pictures[0]) { seen.record($0) }
        XCTAssertEqual(try Data(contentsOf: url), made.original)
        XCTAssertEqual(seen.last, 1)
        // The picture of the answer too.
        let replyURL = try await phone.engine.download(macId: id, asset: reply.pictures[0]) { _ in }
        XCTAssertEqual(try Data(contentsOf: replyURL), made.reply)
    }

    /// A file that was there when the page was read and is gone when it is opened: the Mac says it is not found.
    func testAPictureDeletedAfterThePageWasReadIsNotFoundWhenOpened() async throws {
        try await pair()
        let data = try png(width: 200, height: 150, noise: false)
        let file = try write(data, as: "short-lived.png")
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        try writeHistory([try entry("short", at: now - 60_000, files: [(file, data.count, true)])])
        let picture = try unwrap(try await page().items.first?.pictures.first)
        XCTAssertFalse(picture.gone)
        XCTAssertNotNil(picture.thumb)

        try FileManager.default.removeItem(at: file)
        do {
            _ = try await phone.engine.download(macId: try await macId(), asset: picture) { _ in }
            XCTFail("the file is gone")
        } catch BackendError.unavailable(let reason) {
            XCTAssertEqual(reason, "notFound")
        }
    }

    func testAPictureThatIsTooLargeHasNoThumbnailAndIsRefusedWithoutAsking() async throws {
        try await pair()
        let huge = try write(Data(count: 11 * 1024 * 1024), as: "huge.png")
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        try writeHistory([try entry("huge", at: now - 60_000, files: [(huge, 11 * 1024 * 1024, true)])])
        let item = try unwrap(try await page().items.first)
        let picture = try unwrap(item.pictures.first)
        XCTAssertTrue(picture.tooLarge)
        XCTAssertNil(picture.thumb)
        XCTAssertFalse(picture.gone)
        do {
            _ = try await phone.engine.download(macId: try await macId(), asset: picture) { _ in }
            XCTFail("too large")
        } catch BackendError.tooLarge {}
    }

    // MARK: one exchange

    func testAnExchangeIsReadByItsIdWithTheSamePicturesAsThePage() async throws {
        try await pair()
        let made = try await prepare()
        let fromPage = try unwrap(try await page().items.first)
        let id = try await macId()
        let single = try await phone.engine.sessionExchange(macId: id, id: made.exchangeId)
        XCTAssertEqual(single, fromPage, "the same ids, the same thumbnails: one picture is one picture wherever it is read")
        let missing = try await phone.engine.sessionExchange(macId: id, id: "no-such-record")
        XCTAssertNil(missing, "a record the Mac does not have is nothing, not an error")
    }

    // MARK: a new daemon

    /// The id of a picture is made of the exchange and the place, not remembered: a picture already on the phone's screen
    /// opens after the daemon is replaced.
    func testAPictureOfAnEarlierPageStillOpensAfterTheDaemonRestarts() async throws {
        try await pair()
        let made = try await prepare()
        let picture = try unwrap(try await page().items.first?.pictures.first)
        let id = try await macId()

        _ = try await mac.cli(["daemon", "restart", "--force"], timeout: 60)
        await eventually("the Mac to come back", timeout: 60) { await self.phone.state().macs.first?.link == .online }

        let url = try await phone.engine.download(macId: id, asset: picture) { _ in }
        XCTAssertEqual(try Data(contentsOf: url), made.original)
    }
}
