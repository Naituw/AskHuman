import CoreGraphics
import CryptoKit
import ImageIO
import UniformTypeIdentifiers
import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// docs/specs/ios-attachment-traffic.md against a real daemon: a picture is looked at as a preview first and fetched in
/// full on request, and the same content in a later request is not fetched again. The Mac says what it sent in
/// `trafficBreakdown` (`channel ios status --json`), by message type (`kinds`) and by what the chunks were for
/// (`assets`), so the assertions count messages and bytes on the wire and leave room only for the header of each chunk.
///
///   AH_E2E_BIN=<installed AskHuman> AH_E2E_BROKER_DIR=ios/Tools/local-broker swift test --filter AttachmentTrafficE2ETests
///
/// `AH_E2E_PICTURE=<png or jpeg>` replaces the synthetic screenshot the tests use, to measure a picture of one's own.
/// Run against a Mac of an older release, `testWhateverTheMacOffersTheFileArrivesIntact` is the compatibility check of a
/// phone of this build with that Mac; the others need a Mac that offers previews and skip themselves without one.
final class AttachmentTrafficE2ETests: E2ECase {
    // MARK: the Mac's account

    struct Count: Equatable {
        var msgs = 0
        var bytes = 0
    }

    /// What `trafficBreakdown` of `channel ios status --json` says: by message type (`kinds`) and by what the asset chunks
    /// were for (`parts`, the `assets` map). Shared with the live run on the simulator (`SimulatorLiveTests`).
    struct Ledger {
        var kinds: [String: Count] = [:]
        var parts: [String: Count] = [:]

        init(kinds: [String: Count] = [:], parts: [String: Count] = [:]) {
            self.kinds = kinds
            self.parts = parts
        }

        init(status: [String: Any]) {
            let breakdown = status["trafficBreakdown"] as? [String: Any] ?? [:]
            func counts(_ key: String) -> [String: Count] {
                (breakdown[key] as? [String: [String: Any]] ?? [:]).mapValues {
                    Count(msgs: ($0["msgs"] as? NSNumber)?.intValue ?? 0, bytes: ($0["bytes"] as? NSNumber)?.intValue ?? 0)
                }
            }
            self.init(kinds: counts("kinds"), parts: counts("assets"))
        }

        func kind(_ key: String) -> Count { kinds[key] ?? Count() }
        func part(_ key: String) -> Count { parts[key] ?? Count() }

        /// What the four parts that split the chunks add up to: the same messages `kinds["out:asset.chunk"]` counts.
        var chunks: Count {
            ["chunk.preview", "chunk.preview.repeat", "chunk.original", "chunk.original.repeat"]
                .reduce(into: Count()) { sum, key in
                    sum.msgs += part(key).msgs
                    sum.bytes += part(key).bytes
                }
        }

        /// What was counted since `earlier`.
        func since(_ earlier: Ledger) -> Ledger {
            func difference(_ now: [String: Count], _ then: [String: Count]) -> [String: Count] {
                var result: [String: Count] = [:]
                for key in Set(now.keys).union(then.keys) {
                    let new = now[key] ?? Count(), old = then[key] ?? Count()
                    result[key] = Count(msgs: new.msgs - old.msgs, bytes: new.bytes - old.bytes)
                }
                return result
            }
            return Ledger(kinds: difference(kinds, earlier.kinds), parts: difference(parts, earlier.parts))
        }
    }

    private func ledger() async throws -> Ledger {
        Ledger(status: try await mac.status())
    }

    /// The seven parts the Mac counts asset traffic under, and no others.
    private let knownParts: Set<String> = [
        "chunk.preview", "chunk.preview.repeat", "chunk.original", "chunk.original.repeat",
        "get.request", "get.history", "get.unresolved",
    ]

    /// Bytes of one chunk frame that are not the file: the binary header and tag, the length of the message header and
    /// that header (ids, the file's name, size and checksum). Generous on purpose; a base64 chunk is 65 KB over.
    private let frameOverhead = 600

    private func chunks(_ bytes: Int) -> Int { max(1, (bytes + Wire.assetChunkBytes - 1) / Wire.assetChunkBytes) }

    // MARK: pictures and requests

    /// A screenshot of about 3 MB, made once per run (generating it takes seconds in a debug build).
    private static let screenshot: Data = {
        do { return try Pictures.png(width: 2880, height: 1800, noise: Pictures.screenshotNoise) } catch { fatalError("\(error)") }
    }()

    /// The picture a test looks at: the synthetic screenshot, or the file `AH_E2E_PICTURE` names.
    private var picture: (data: Data, ext: String) {
        if let path = ProcessInfo.processInfo.environment["AH_E2E_PICTURE"], !path.isEmpty,
           let data = FileManager.default.contents(atPath: path) {
            return (data, URL(fileURLWithPath: path).pathExtension)
        }
        return (Self.screenshot, "png")
    }

    private func write(_ data: Data, as name: String) throws -> URL {
        let url = mac.home.appendingPathComponent(name)
        try data.write(to: url)
        return url
    }

    private func sha256(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }

    /// Pixel size of a picture ImageIO opens, which includes that it decodes.
    private func pixelSize(of data: Data) throws -> (width: Int, height: Int) {
        let source = try unwrap(CGImageSourceCreateWithData(data as CFData, nil))
        let properties = try unwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        XCTAssertNotNil(CGImageSourceCreateImageAtIndex(source, 0, nil), "the picture does not decode")
        return (
            try unwrap((properties[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue),
            try unwrap((properties[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue)
        )
    }

    /// Asks with `file` attached and returns the request as it shows on the phone, content and all.
    private func ask(about file: URL) async throws -> (cmd: Cmd, item: InboxItem, asset: AssetRef) {
        let known = Set(await phone.state().inbox.map(\.requestId))
        let cmd = try mac.ask(["Look at the picture", "-f", file.path, "-q", "Fine?", "-o", "Yes"])
        await eventually("the new request with its content on the phone") {
            await self.phone.state().inbox.contains { !known.contains($0.requestId) && $0.view != nil }
        }
        let item = try unwrap(await phone.state().inbox.first { !known.contains($0.requestId) })
        let asset = try unwrap(item.view?.ask?.message.files.first)
        return (cmd, item, asset)
    }

    private func answer(_ cmd: Cmd, _ item: InboxItem) async throws {
        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0])]
        )
        await assertFinishes(cmd)
    }

    private var assetDirectory: URL { phone.root.appendingPathComponent("assets") }

    private func cachedNames() -> [String] {
        ((try? FileManager.default.contentsOfDirectory(atPath: assetDirectory.path)) ?? []).sorted()
    }

    /// The phone loses what it had of the pictures, as it does when the system clears its caches.
    private func loseTheCache() throws {
        for name in cachedNames() { try FileManager.default.removeItem(at: assetDirectory.appendingPathComponent(name)) }
    }

    /// CPU time the daemon of this Mac has used so far, in seconds (what `ps` calls TIME, a hundredth of a second fine).
    private func daemonCPUSeconds() async throws -> Double {
        // `run` alone would be XCTestCase's own method.
        let list = try await AHE2ETests.run("/bin/ps", ["-axo", "pid=,command="], env: [:], cwd: mac.home, timeout: 20)
        var total = 0.0
        for line in list.stdout.split(whereSeparator: \.isNewline) where line.contains(environment.askHumanBinary) {
            guard let pid = line.split(separator: " ").first.flatMap({ Int32($0) }) else { continue }
            let detail = try await AHE2ETests.run("/bin/ps", ["eww", "-p", "\(pid)"], env: [:], cwd: mac.home, timeout: 20)
            guard detail.stdout.contains("ASKHUMAN_HOME=\(mac.configDir.path)") else { continue }
            let time = try await AHE2ETests.run("/bin/ps", ["-o", "time=", "-p", "\(pid)"], env: [:], cwd: mac.home, timeout: 20)
            total += Self.seconds(inCPUTime: time.stdout.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return total
    }

    /// `1:02.34`, `0:00.10` or `1:02:03` as seconds.
    private static func seconds(inCPUTime text: String) -> Double {
        text.split(separator: ":").reduce(0.0) { $0 * 60 + (Double($1) ?? 0) }
    }

    private func requirePreviews() async throws {
        let caps = try unwrap(await phone.state().macs.first).caps
        try XCTSkipUnless(caps.assetPreview, "this Mac does not offer previews")
    }

    // MARK: acceptance

    func testAScreenshotComesAsAPreviewAndTheOriginalOnRequest() async throws {
        try await pair()
        try await requirePreviews()
        let (original, ext) = picture
        XCTAssertGreaterThan(original.count, 400 * 1024)
        let file = try write(original, as: "screenshot.\(ext)")
        let start = try await ledger()
        let (cmd, item, asset) = try await ask(about: file)
        XCTAssertTrue(asset.preview, "the Mac offers a preview of a picture this size")
        XCTAssertEqual(asset.sha256, sha256(original), "the request names the file by its hash")
        XCTAssertEqual(asset.size, UInt64(original.count))

        // 1. Looking at the picture fetches its preview and nothing else.
        let viewing = try await phone.engine.downloadForViewing(macId: item.macId, asset: asset) { _ in }
        XCTAssertTrue(viewing.isPreview)
        let preview = try Data(contentsOf: viewing.url)
        XCTAssertEqual(preview.subdata(in: 4..<12), Data("ftypavif".utf8), "an AVIF file")
        let shown = try pixelSize(of: preview)
        let source = try pixelSize(of: original)
        XCTAssertLessThanOrEqual(max(shown.width, shown.height), 1600, "the long edge is at most 1600 px")
        XCTAssertLessThan(preview.count * 8, original.count, "\(preview.count) B for \(original.count) B")
        XCTAssertEqual(
            Double(shown.width) / Double(shown.height), Double(source.width) / Double(source.height), accuracy: 0.01,
            "the same picture, scaled"
        )
        let afterPreview = try await ledger()
        let first = afterPreview.since(start)
        XCTAssertEqual(first.part("chunk.preview").msgs, chunks(preview.count))
        XCTAssertGreaterThanOrEqual(first.part("chunk.preview").bytes, preview.count)
        XCTAssertLessThan(first.part("chunk.preview").bytes, preview.count + chunks(preview.count) * frameOverhead)
        XCTAssertEqual(first.part("chunk.original"), Count(), "no original went out")
        XCTAssertEqual(first.part("chunk.preview.repeat"), Count())
        XCTAssertEqual(first.part("get.request").msgs, 1)
        XCTAssertEqual(first.kind("in:asset.get").msgs, 1)
        XCTAssertEqual(first.chunks, first.kind("out:asset.chunk"), "the parts of the chunks add up to what the type says")

        // 2. The original on request is the file, byte for byte.
        let url = try await phone.engine.download(macId: item.macId, asset: asset) { _ in }
        XCTAssertEqual(try Data(contentsOf: url), original, "the original is the Mac's file, byte for byte")
        let afterOriginal = try await ledger()
        let second = afterOriginal.since(afterPreview)
        XCTAssertEqual(second.part("chunk.original").msgs, chunks(original.count))
        XCTAssertGreaterThanOrEqual(second.part("chunk.original").bytes, original.count)
        XCTAssertLessThan(second.part("chunk.original").bytes, original.count + chunks(original.count) * frameOverhead)
        XCTAssertEqual(second.part("chunk.preview"), Count())
        XCTAssertEqual(second.part("chunk.original.repeat"), Count())
        XCTAssertGreaterThanOrEqual(second.part("get.request").msgs, 1)
        XCTAssertEqual(second.chunks, second.kind("out:asset.chunk"))

        // 3. The status says all this under the parts the specification names, and under no others.
        let all = afterOriginal
        XCTAssertTrue(Set(all.parts.keys).isSubset(of: knownParts), "unexpected parts: \(all.parts.keys.sorted())")
        XCTAssertEqual(all.chunks, all.kind("out:asset.chunk"), "every byte in one of the parts")
        XCTAssertEqual(all.part("get.request").msgs, all.kind("in:asset.get").msgs, "every asset.get named a file of a request")
        XCTAssertEqual(all.part("get.unresolved"), Count())

        try await answer(cmd, item)
        let wireOfPreview = first.part("chunk.preview").bytes
        report([
            "originalBytes": original.count, "previewBytes": preview.count, "previewOnWire": wireOfPreview,
            "originalOnWire": second.part("chunk.original").bytes,
            "previewPixels": "\(shown.width)x\(shown.height)", "originalPixels": "\(source.width)x\(source.height)",
        ])
    }

    func testTheSameScreenshotInALaterRequestIsNotFetchedAgain() async throws {
        try await pair()
        try await requirePreviews()
        let (original, ext) = picture

        // The first request: the preview.
        let a = try await ask(about: try write(original, as: "first.\(ext)"))
        let viewed = try await phone.engine.downloadForViewing(macId: a.item.macId, asset: a.asset) { _ in }
        XCTAssertTrue(viewed.isPreview)
        let preview = try Data(contentsOf: viewed.url)
        try await answer(a.cmd, a.item)
        let afterA = try await ledger()

        // Another request with the same content under another asset id: the preview is on the phone, so nothing is asked.
        let b = try await ask(about: try write(original, as: "second.\(ext)"))
        XCTAssertNotEqual(b.asset.assetId, a.asset.assetId)
        XCTAssertEqual(b.asset.sha256, a.asset.sha256)
        let again = try await phone.engine.downloadForViewing(macId: b.item.macId, asset: b.asset) { _ in }
        XCTAssertTrue(again.isPreview)
        XCTAssertEqual(try Data(contentsOf: again.url), preview)
        let afterB = try await ledger()
        XCTAssertEqual(afterB.since(afterA).kind("in:asset.get"), Count(), "nothing was asked of the Mac")
        XCTAssertEqual(afterB.since(afterA).chunks, Count(), "and nothing was sent")

        // The original is fetched once, and then the phone has it for every request.
        let full = try await phone.engine.download(macId: b.item.macId, asset: b.asset) { _ in }
        XCTAssertEqual(try Data(contentsOf: full), original)
        try await answer(b.cmd, b.item)
        let afterFull = try await ledger()
        XCTAssertEqual(afterFull.since(afterB).part("chunk.original").msgs, chunks(original.count))
        XCTAssertEqual(afterFull.since(afterB).part("chunk.original.repeat"), Count(), "the first time is not a repeat")

        let c = try await ask(about: try write(original, as: "third.\(ext)"))
        let seen = try await phone.engine.downloadForViewing(macId: c.item.macId, asset: c.asset) { _ in }
        XCTAssertFalse(seen.isPreview, "the original is here, so that is what is shown")
        XCTAssertEqual(try Data(contentsOf: seen.url), original)
        let exported = try await phone.engine.download(macId: c.item.macId, asset: c.asset) { _ in }
        XCTAssertEqual(try Data(contentsOf: exported), original)
        let afterC = try await ledger()
        XCTAssertEqual(afterC.since(afterFull).kind("in:asset.get"), Count(), "the third request cost nothing")
        XCTAssertEqual(afterC.since(afterFull).chunks, Count())
        try await answer(c.cmd, c.item)

        // The phone loses its files (the system cleared its caches): fetching again is counted as a repeat.
        XCTAssertTrue(cachedNames().contains { $0.count == 64 }, "the phone kept the content under its hash")
        try loseTheCache()
        let d = try await ask(about: try write(original, as: "fourth.\(ext)"))
        let refetched = try await phone.engine.downloadForViewing(macId: d.item.macId, asset: d.asset) { _ in }
        XCTAssertTrue(refetched.isPreview)
        _ = try await phone.engine.download(macId: d.item.macId, asset: d.asset) { _ in }
        let afterD = try await ledger()
        let repeated = afterD.since(afterC)
        XCTAssertEqual(repeated.part("chunk.preview"), Count())
        XCTAssertEqual(repeated.part("chunk.original"), Count())
        XCTAssertEqual(repeated.part("chunk.preview.repeat").msgs, chunks(preview.count))
        XCTAssertEqual(repeated.part("chunk.original.repeat").msgs, chunks(original.count))
        XCTAssertEqual(afterD.chunks, afterD.kind("out:asset.chunk"))
        try await answer(d.cmd, d.item)

        report([
            "repeatedPreviewBytes": repeated.part("chunk.preview.repeat").bytes,
            "repeatedOriginalBytes": repeated.part("chunk.original.repeat").bytes,
            "getsForFourRequests": afterD.part("get.request").msgs,
        ])
    }

    func testPicturesThatAreNotPreviewedComeAsTheFile() async throws {
        try await pair()
        try await requirePreviews()

        // Small: the Mac does not offer a preview, and the phone does not ask for one.
        let small = try Pictures.png(width: 640, height: 400, noise: 0)
        XCTAssertLessThan(small.count, 300 * 1024)
        let smallRequest = try await ask(about: try write(small, as: "small.png"))
        XCTAssertFalse(smallRequest.asset.preview)
        let smallViewing = try await phone.engine.downloadForViewing(
            macId: smallRequest.item.macId, asset: smallRequest.asset
        ) { _ in }
        XCTAssertFalse(smallViewing.isPreview)
        XCTAssertEqual(try Data(contentsOf: smallViewing.url), small)
        try await answer(smallRequest.cmd, smallRequest.item)

        // With a transparent pixel: offered by its type and size, then answered with the file (`variant: original`),
        // which the phone takes as it is.
        let transparent = try Pictures.png(width: 2880, height: 1800, noise: Pictures.screenshotNoise, transparentCorner: true)
        let before = try await ledger()
        let transparentRequest = try await ask(about: try write(transparent, as: "transparent.png"))
        XCTAssertTrue(transparentRequest.asset.preview)
        let transparentViewing = try await phone.engine.downloadForViewing(
            macId: transparentRequest.item.macId, asset: transparentRequest.asset
        ) { _ in }
        XCTAssertFalse(transparentViewing.isPreview, "a picture with transparency is shown as it is")
        XCTAssertEqual(try Data(contentsOf: transparentViewing.url), transparent)
        let after = try await ledger()
        XCTAssertEqual(after.since(before).part("chunk.preview"), Count())
        XCTAssertEqual(after.since(before).part("chunk.original").msgs, chunks(transparent.count))
        try await answer(transparentRequest.cmd, transparentRequest.item)

        // A JPEG gets a preview like a PNG does.
        let jpeg = try Pictures.jpeg(width: 2880, height: 1800, noise: Pictures.screenshotNoise)
        XCTAssertGreaterThan(jpeg.count, 450 * 1024)
        let jpegRequest = try await ask(about: try write(jpeg, as: "photo.jpg"))
        XCTAssertTrue(jpegRequest.asset.preview)
        let jpegViewing = try await phone.engine.downloadForViewing(macId: jpegRequest.item.macId, asset: jpegRequest.asset) { _ in }
        XCTAssertTrue(jpegViewing.isPreview)
        let shown = try pixelSize(of: try Data(contentsOf: jpegViewing.url))
        XCTAssertLessThanOrEqual(max(shown.width, shown.height), 1600)
        try await answer(jpegRequest.cmd, jpegRequest.item)
    }

    func testAFileChangedAfterTheRequestIsServedAsTheFileAndKeptOnlyUnderItsAssetId() async throws {
        try await pair()
        try await requirePreviews()
        // Two files of the same size that differ in a comment of the PNG: the preview could be made of either, and only
        // the hash tells which one the request was built with.
        let before = try Pictures.png(width: 2880, height: 1800, noise: Pictures.screenshotNoise, comment: String(repeating: "a", count: 32))
        let after = try Pictures.png(width: 2880, height: 1800, noise: Pictures.screenshotNoise, comment: String(repeating: "b", count: 32))
        XCTAssertEqual(before.count, after.count)
        XCTAssertNotEqual(before, after)
        let file = try write(before, as: "changing.png")
        let start = try await ledger()
        let (cmd, item, asset) = try await ask(about: file)
        XCTAssertEqual(asset.sha256, sha256(before))
        try after.write(to: file)

        let viewing = try await phone.engine.downloadForViewing(macId: item.macId, asset: asset) { _ in }
        XCTAssertFalse(viewing.isPreview, "the Mac does not hand out the preview of a file the request was not built with")
        XCTAssertEqual(try Data(contentsOf: viewing.url), after, "the file as it is now")
        XCTAssertFalse(cachedNames().contains { $0.count == 64 }, "nothing is kept under a hash: \(cachedNames())")
        let counted = try await ledger().since(start)
        XCTAssertEqual(counted.part("chunk.preview"), Count())
        XCTAssertEqual(counted.part("chunk.original").msgs, chunks(after.count))
        try await answer(cmd, item)
    }

    func testWhateverTheMacOffersTheFileArrivesIntact() async throws {
        try await pair()
        let caps = try unwrap(await phone.state().macs.first).caps
        let (original, ext) = picture
        let file = try write(original, as: "screenshot.\(ext)")
        let start = try await ledger()
        let (cmd, item, asset) = try await ask(about: file)
        if caps.assetPreview {
            XCTAssertTrue(asset.preview)
            XCTAssertEqual(asset.sha256, sha256(original))
        } else {
            XCTAssertFalse(asset.preview, "a Mac that makes no previews offers none")
            XCTAssertNil(asset.sha256)
        }

        let viewing = try await phone.engine.downloadForViewing(macId: item.macId, asset: asset) { _ in }
        XCTAssertEqual(viewing.isPreview, caps.assetPreview, "a preview comes when the Mac offers them, the file when it does not")
        let previewBytes = viewing.isPreview ? try Data(contentsOf: viewing.url).count : 0
        if !viewing.isPreview { XCTAssertEqual(try Data(contentsOf: viewing.url), original) }
        let url = try await phone.engine.download(macId: item.macId, asset: asset) { _ in }
        XCTAssertEqual(try Data(contentsOf: url), original)

        // The wire format is the one the Mac said it opens: raw bytes in blob frames, or, for a Mac that says nothing,
        // JSON with the bytes in base64. Everything the phone fetched is on it once: the preview if it came, the file.
        let counted = try await ledger().since(start)
        let sent = counted.kind("out:asset.chunk")
        let fetched = previewBytes + original.count
        if counted.kinds.isEmpty {
            print("[e2e] this Mac keeps no traffic breakdown by type")
        } else if caps.binary {
            XCTAssertGreaterThanOrEqual(sent.bytes, fetched)
            XCTAssertLessThan(sent.bytes, fetched + sent.msgs * frameOverhead, "raw bytes: \(sent.bytes) B for \(fetched) B")
        } else {
            XCTAssertGreaterThan(sent.bytes, fetched * 13 / 10, "JSON with base64: \(sent.bytes) B for \(fetched) B")
        }
        try await answer(cmd, item)
        report([
            "macOffersPreview": caps.assetPreview, "macOffersBinary": caps.binary, "chunkBytesOnWire": sent.bytes,
            "originalBytes": original.count, "viewedAsPreview": viewing.isPreview,
        ])
    }

    /// Acceptance 15: the preview is made in the time the build settings are meant to give, and does not cost the daemon
    /// much CPU. Measured, not asserted, except for a tripwire that catches the dependencies being built without
    /// optimisation (about 5 s a picture instead of about 0.3 s).
    func testThePreviewIsMadeInTimeAndDoesNotCostMuchCPU() async throws {
        try await pair()
        try await requirePreviews()
        let (original, ext) = picture
        let (cmd, item, asset) = try await ask(about: try write(original, as: "screenshot.\(ext)"))

        let cpuBefore = try await daemonCPUSeconds()
        let began = Date()
        let cold = try await phone.engine.downloadForViewing(macId: item.macId, asset: asset) { _ in }
        let coldSeconds = Date().timeIntervalSince(began)
        let cpuCold = try await daemonCPUSeconds() - cpuBefore
        XCTAssertTrue(cold.isPreview)
        let preview = try Data(contentsOf: cold.url)

        // The same preview again, from the Mac's cache: the phone's own files are gone.
        try loseTheCache()
        let againBegan = Date()
        let warm = try await phone.engine.downloadForViewing(macId: item.macId, asset: asset) { _ in }
        let warmSeconds = Date().timeIntervalSince(againBegan)
        XCTAssertTrue(warm.isPreview)
        XCTAssertEqual(try Data(contentsOf: warm.url), preview)

        report([
            "originalBytes": original.count, "previewBytes": preview.count,
            "firstPreviewSeconds": (coldSeconds * 1000).rounded() / 1000,
            "cachedPreviewSeconds": (warmSeconds * 1000).rounded() / 1000,
            "daemonCPUSecondsForTheFirstPreview": (cpuCold * 100).rounded() / 100,
        ])
        XCTAssertLessThan(coldSeconds, 4, "the first preview of \(original.count) B took \(coldSeconds) s")
        try await answer(cmd, item)
    }
}

/// The synthetic pictures keep the sizes the tests above assume.
final class SyntheticPicturesTests: XCTestCase {
    func testThePicturesAreOfTheSizesTheE2ETestsAssume() throws {
        // Generating them takes seconds in a debug build, and only the end-to-end run needs them.
        try XCTSkipUnless(ProcessInfo.processInfo.environment["AH_E2E_BIN"]?.isEmpty == false, "AH_E2E_BIN is not set")
        let screenshot = try Pictures.png(width: 2880, height: 1800, noise: Pictures.screenshotNoise)
        XCTAssertGreaterThan(screenshot.count, 2_500_000, "about 3 MB: \(screenshot.count)")
        XCTAssertLessThan(screenshot.count, 4_000_000, "about 3 MB: \(screenshot.count)")
        XCTAssertLessThan(try Pictures.png(width: 640, height: 400, noise: 0).count, 300 * 1024)
        let transparent = try Pictures.png(width: 2880, height: 1800, noise: Pictures.screenshotNoise, transparentCorner: true)
        XCTAssertGreaterThan(transparent.count, 400 * 1024)
        XCTAssertGreaterThan(try Pictures.jpeg(width: 2880, height: 1800, noise: Pictures.screenshotNoise).count, 450 * 1024)
        // A comment is all that tells these two apart, so a changed file is a file of the same size.
        let first = try Pictures.png(width: 640, height: 400, noise: 0, comment: String(repeating: "a", count: 32))
        let second = try Pictures.png(width: 640, height: 400, noise: 0, comment: String(repeating: "b", count: 32))
        XCTAssertEqual(first.count, second.count)
        XCTAssertNotEqual(first, second)
    }
}

/// Deterministic pictures an encoder takes for a screenshot: flat areas, runs of small rectangles that stand for text,
/// and one block of photographic detail. Opaque with an alpha channel, as macOS writes screenshots.
enum Pictures {
    /// How much grain the photographic block gets so that a 2880 x 1800 screenshot is about 3 MB as a PNG.
    static let screenshotNoise = 14

    private struct Xorshift {
        var state: UInt64 = 0x9E37_79B9_7F4A_7C15
        mutating func next() -> UInt64 {
            state ^= state << 13
            state ^= state >> 7
            state ^= state << 17
            return state
        }

        mutating func below(_ bound: Int) -> Int { Int(next() % UInt64(bound)) }
    }

    private static func image(width: Int, height: Int, noise: Int, transparentCorner: Bool) throws -> CGImage {
        var rng = Xorshift()
        var pixels = [UInt8](repeating: 255, count: width * height * 4)
        pixels.withUnsafeMutableBufferPointer { p in
            func fill(_ x0: Int, _ y0: Int, _ x1: Int, _ y1: Int, _ r: Int, _ g: Int, _ b: Int) {
                let left = max(0, x0), right = min(width, x1), top = max(0, y0), bottom = min(height, y1)
                guard left < right, top < bottom else { return }
                for y in top..<bottom {
                    var index = (y * width + left) * 4
                    for _ in left..<right {
                        p[index] = UInt8(r)
                        p[index + 1] = UInt8(g)
                        p[index + 2] = UInt8(b)
                        p[index + 3] = 255
                        index += 4
                    }
                }
            }
            let unit = max(1, width / 1440)
            for y in 0..<height {
                let shade = 246 - y * 14 / height
                fill(0, y, width, y + 1, shade, shade + 1, shade + 3)
            }
            fill(0, 0, width, 28 * unit, 226, 228, 232)
            for light in 0..<3 {
                fill((12 + light * 18) * unit, 9 * unit, (22 + light * 18) * unit, 19 * unit, 255 - light * 30, 95 + light * 60, 80)
            }
            let sidebar = width / 6
            fill(0, 28 * unit, sidebar, height, 238, 239, 243)
            var y = 50 * unit
            while y < height - 30 * unit {
                fill(20 * unit, y, 20 * unit + 60 * unit + rng.below(120 * unit), y + 8 * unit, 92, 97, 108)
                y += 26 * unit
            }
            let textBottom = height * 21 / 50
            y = 70 * unit
            while y + 8 * unit < textBottom {
                var x = sidebar + 40 * unit
                while x < width - 70 * unit {
                    let run = 20 * unit + rng.below(90 * unit)
                    fill(x, y, min(x + run, width - 60 * unit), y + 8 * unit, 28, 30, 36)
                    x += run + 9 * unit
                }
                y += 22 * unit
            }
            let left = width * 5 / 12, top = height * 21 / 50, right = width - width / 36, bottom = height - height / 18
            for y in top..<bottom {
                for x in left..<right {
                    let base = 128.0 + 70 * sin(Double(x) / 90) + 50 * sin(Double(y) / 70 + Double(x) / 200)
                    let index = (y * width + x) * 4
                    for channel in 0..<3 {
                        let tint = Double(channel) * 14 * cos(Double(x + y) / 120)
                        let grain = noise > 0 ? rng.below(noise + 1) - noise / 2 : 0
                        p[index + channel] = UInt8(max(0, min(255, Int(base + tint) + grain)))
                    }
                }
            }
            if transparentCorner {
                // Half transparent grey, premultiplied.
                for y in (40 * unit)..<(56 * unit) {
                    for x in (width - 56 * unit)..<(width - 40 * unit) {
                        let index = (y * width + x) * 4
                        p[index] = 100
                        p[index + 1] = 100
                        p[index + 2] = 100
                        p[index + 3] = 128
                    }
                }
            }
        }
        let provider = try XCTUnwrap(CGDataProvider(data: Data(pixels) as CFData))
        let image = CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
            space: try XCTUnwrap(CGColorSpace(name: CGColorSpace.sRGB)),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue), provider: provider,
            decode: nil, shouldInterpolate: false, intent: .defaultIntent
        )
        return try XCTUnwrap(image)
    }

    private static func encode(_ image: CGImage, as type: UTType, _ properties: [CFString: Any]) throws -> Data {
        let data = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(data, type.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, properties as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw E2EError("could not encode a picture") }
        return data as Data
    }

    static func png(width: Int, height: Int, noise: Int, comment: String = "", transparentCorner: Bool = false) throws -> Data {
        var properties: [CFString: Any] = [:]
        if !comment.isEmpty { properties[kCGImagePropertyPNGDictionary] = [kCGImagePropertyPNGDescription: comment] }
        return try encode(
            try image(width: width, height: height, noise: noise, transparentCorner: transparentCorner), as: .png, properties
        )
    }

    static func jpeg(width: Int, height: Int, noise: Int) throws -> Data {
        // JPEG has no alpha channel: ImageIO flattens it.
        try encode(
            try image(width: width, height: height, noise: noise, transparentCorner: false), as: .jpeg,
            [kCGImageDestinationLossyCompressionQuality: 0.92]
        )
    }
}
