import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// docs/specs/ios-attachment-traffic.md §5 against a real daemon: a file goes both ways as raw bytes in blob frames. The
/// Mac says what it sent and received in `trafficBreakdown` (`channel ios status --json`), counted on the wire, so the
/// assertions leave room only for the header of each chunk: base64 inside JSON would cost a third more than the file.
///
///   AH_E2E_BIN=<installed AskHuman> AH_E2E_BROKER_DIR=ios/Tools/local-broker swift test --filter BinaryTransportE2ETests
final class BinaryTransportE2ETests: E2ECase {
    private struct Kind: Equatable {
        var msgs = 0
        var bytes = 0
    }

    /// The Mac's own account of what it sent and received by message type this month.
    private func breakdown() async throws -> [String: Kind] {
        let status = try await mac.status()
        let kinds = (status["trafficBreakdown"] as? [String: Any])?["kinds"] as? [String: [String: Any]] ?? [:]
        return kinds.mapValues {
            Kind(msgs: ($0["msgs"] as? NSNumber)?.intValue ?? 0, bytes: ($0["bytes"] as? NSNumber)?.intValue ?? 0)
        }
    }

    private func grew(_ key: String, from before: [String: Kind], to after: [String: Kind]) -> Kind {
        let old = before[key] ?? Kind(), new = after[key] ?? Kind()
        return Kind(msgs: new.msgs - old.msgs, bytes: new.bytes - old.bytes)
    }

    /// Bytes of one chunk frame that are not the file: the binary header and tag, the length of the message header and
    /// that header (ids, the file's name, size and checksum). Generous on purpose; a base64 chunk is 65 KB over.
    private let frameOverhead = 600

    func testAFileGoesBothWaysAsRawBytesAndArrivesIntact() async throws {
        try await pair()
        let macState = await phone.state().macs.first
        XCTAssertEqual(macState?.caps.binary, true, "the Mac offers binary to a phone that said it opens it")

        // Mac to phone: four chunks (the last one short), which the Mac sends as blob frames.
        let file = mac.home.appendingPathComponent("blob.bin")
        let download = Data((0..<600_000).map { _ in UInt8.random(in: 0...255) })
        try download.write(to: file)
        let start = try await breakdown()
        let ask = try mac.ask(["See the file", "-f", file.path, "-q", "Fine?", "-o", "Yes"])
        let item = try await phone.request()
        let asset = try unwrap(item.view?.ask?.message.files.first)
        XCTAssertEqual(asset.size, UInt64(download.count))
        let url = try await phone.engine.download(macId: item.macId, asset: asset) { _ in }
        XCTAssertEqual(try Data(contentsOf: url), download, "the file arrived intact, checksum verified by the phone")
        let afterDownload = try await breakdown()
        let down = grew("out:asset.chunk", from: start, to: afterDownload)
        let chunks = (download.count + Wire.assetChunkBytes - 1) / Wire.assetChunkBytes
        XCTAssertEqual(down.msgs, chunks)
        XCTAssertGreaterThanOrEqual(down.bytes, download.count)
        XCTAssertLessThan(down.bytes, download.count + chunks * frameOverhead, "\(down.bytes) B on the wire for \(download.count) B")

        // Phone to Mac: the same, in the other direction.
        let upload = Data((0..<400_000).map { _ in UInt8.random(in: 0...255) })
        let uploadId = try await phone.engine.upload(
            macId: item.macId, requestId: item.requestId,
            source: UploadSource(name: "reply.bin", mime: "application/octet-stream", data: upload, isImage: false)
        ) { _ in }
        let afterUpload = try await breakdown()
        let up = grew("in:upload.chunk", from: afterDownload, to: afterUpload)
        let uploadChunks = (upload.count + Wire.assetChunkBytes - 1) / Wire.assetChunkBytes
        XCTAssertEqual(up.msgs, uploadChunks)
        XCTAssertGreaterThanOrEqual(up.bytes, upload.count)
        XCTAssertLessThan(up.bytes, upload.count + uploadChunks * frameOverhead, "\(up.bytes) B on the wire for \(upload.count) B")
        XCTAssertEqual(grew("rejected", from: start, to: afterUpload).msgs, 0, "the Mac refused nothing the phone sent")

        // The file is on the Mac, byte for byte: the reply carries it.
        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0], files: [uploadId])]
        )
        await assertFinishes(ask)
        let line = try unwrap(ask.stdout.split(separator: "\n").map(String.init).first { $0.contains("reply.bin") })
        let path = line.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "- ", with: "")
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: path)), upload)

        report([
            "downloadFile": download.count, "downloadWire": down.bytes, "downloadChunks": down.msgs,
            "uploadFile": upload.count, "uploadWire": up.bytes, "uploadChunks": up.msgs,
        ])
    }
}
