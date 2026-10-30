import XCTest
import AVFoundation
import CoreVideo
import AHProtocol
import AHStore
@testable import AHCore

@MainActor final class VideoAttachmentTests: XCTestCase {
    func testPreflightChoosesQualityWithHeadroomWithoutEncoding() throws {
        let choices = [
            VideoAttachmentPreparer.Estimate(preset: "1080", bytes: 20_000),
            .init(preset: "720", bytes: 8_000),
            .init(preset: "480", bytes: 4_000),
            .init(preset: "low", bytes: 2_000),
        ]
        XCTAssertEqual(try VideoAttachmentPreparer.choose(choices, limit: 10_000).preset, "720")
        XCTAssertEqual(try VideoAttachmentPreparer.choose([.init(preset: "boundary", bytes: 8_000)], limit: 10_000).preset, "boundary")
        for bytes in [8_001, 9_000, 9_500, 10_000] {
            XCTAssertThrowsError(try VideoAttachmentPreparer.choose([.init(preset: "edge", bytes: Int64(bytes))], limit: 10_000)) {
                XCTAssertEqual($0 as? VideoAttachmentError, .cannotFit)
            }
        }
        XCTAssertThrowsError(try VideoAttachmentPreparer.choose(choices, limit: 1_000)) {
            XCTAssertEqual($0 as? VideoAttachmentError, .cannotFit)
        }
        XCTAssertThrowsError(try VideoAttachmentPreparer.choose([.init(preset: "unknown", bytes: 0)], limit: 10_000)) {
            XCTAssertEqual($0 as? VideoAttachmentError, .unreadable)
        }
    }

    func testSmallVideosKeepTheirBytesNameAndType() async throws {
        let directory = try directory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("Capture.MOV")
        let bytes = Data(repeating: 42, count: 123)
        try bytes.write(to: file)
        let result = try await VideoAttachmentPreparer.prepare(url: file) { _ in
            XCTFail("a video that already fits must not be inspected or encoded")
        }
        XCTAssertEqual(result.data, bytes)
        XCTAssertEqual(result.name, "Capture.MOV")
        XCTAssertEqual(result.mime, "video/quicktime")
        XCTAssertFalse(result.compressed)
    }

    func testAnswerAndInterjectWaitForEveryPreparationToFinish() {
        let answer = AnswerComposer(key: "video", ask: ask([question("q")]), drafts: MemoryDraftStore())
        let interject = InterjectComposer(key: "video")
        interject.text = "Here is the video"
        let first = UUID(), second = UUID()
        for token in [first, second] {
            answer.setAttachmentPreparation(token, active: true)
            interject.setAttachmentPreparation(token, active: true)
        }
        answer.setAttachmentPreparation(first, active: false)
        interject.setAttachmentPreparation(first, active: false)
        XCTAssertFalse(answer.canSend)
        XCTAssertFalse(interject.canSend)
        answer.setAttachmentPreparation(second, active: false)
        interject.setAttachmentPreparation(second, active: false)
        XCTAssertTrue(answer.canSend)
        XCTAssertTrue(interject.canSend)
        // Duplicate completion is harmless and must not unblock another job.
        answer.setAttachmentPreparation(first, active: false)
        answer.setAttachmentPreparation(second, active: true)
        answer.setAttachmentPreparation(first, active: false)
        XCTAssertFalse(answer.canSend)
    }

    func testVideoAttachmentsUseTheFileFieldAndKeepTheirBytes() throws {
        let answer = AnswerComposer(key: "video", ask: ask([question("q")]), drafts: MemoryDraftStore())
        let bytes = Data([1, 2, 3])
        let video = try answer.addAttachment(name: "Video.mp4", mime: "video/mp4", data: bytes, isImage: false, question: 0)
        XCTAssertEqual(try Data(contentsOf: answer.url(of: video)), bytes)
        XCTAssertEqual(answer.items(uploads: [video.id: "uploaded"]).first?.files, ["uploaded"])
        XCTAssertEqual(answer.items(uploads: [video.id: "uploaded"]).first?.images, [])
    }

    func testRealVideoIsCompressedWithoutShorteningAndImpossibleVideoSkipsExport() async throws {
        let directory = try directory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = try await movie(in: directory)
        let size = try source.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        XCTAssertGreaterThan(size, 512 * 1024)
        var didEncode = false
        do {
            _ = try await VideoAttachmentPreparer.prepare(url: source, limit: 8) { stage in
                if case .compressing = stage { didEncode = true }
            }
            XCTFail("an impossible size budget must be rejected before encoding")
        } catch {
            XCTAssertEqual(error as? VideoAttachmentError, .cannotFit)
        }
        XCTAssertFalse(didEncode)
        let result = try await VideoAttachmentPreparer.prepare(url: source, limit: 512 * 1024)
        XCTAssertTrue(result.compressed)
        XCTAssertEqual(result.mime, "video/mp4")
        XCTAssertEqual(result.name, "source.mp4")
        XCTAssertLessThanOrEqual(result.data.count, 512 * 1024)
        let output = directory.appendingPathComponent("result.mp4")
        try result.data.write(to: output)
        let before = try await AVURLAsset(url: source).load(.duration)
        let after = try await AVURLAsset(url: output).load(.duration)
        XCTAssertEqual(before.seconds, after.seconds, accuracy: 0.1)
        let tracks = try await AVURLAsset(url: output).loadTracks(withMediaType: .video)
        XCTAssertFalse(tracks.isEmpty)
    }

    func testCancellingPreparationDoesNotReturnAnAttachment() async throws {
        let directory = try directory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = try await movie(in: directory)
        var job: Task<VideoAttachmentPreparer.Output, Error>?
        job = Task {
            try await VideoAttachmentPreparer.prepare(url: source, limit: 512 * 1024) { stage in
                if case .compressing = stage { job?.cancel() }
            }
        }
        do {
            _ = try await job!.value
            XCTFail("a cancelled preparation must not add a late attachment")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
    }

    private func directory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ah-video-test-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    /// Deterministic changing noise prevents the source from collapsing to a tiny static clip.
    private func movie(in directory: URL) async throws -> URL {
        let url = directory.appendingPathComponent("source.mov")
        let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
        let width = 640, height = 360
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: width, AVVideoHeightKey: height,
            AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: 12_000_000],
        ])
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32ARGB,
            kCVPixelBufferWidthKey as String: width, kCVPixelBufferHeightKey as String: height,
        ])
        writer.add(input)
        XCTAssertTrue(writer.startWriting())
        writer.startSession(atSourceTime: .zero)
        for frame in 0..<30 {
            while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(2)) }
            var buffer: CVPixelBuffer?
            XCTAssertEqual(CVPixelBufferCreate(nil, width, height, kCVPixelFormatType_32ARGB, nil, &buffer), kCVReturnSuccess)
            let pixel = try XCTUnwrap(buffer)
            CVPixelBufferLockBaseAddress(pixel, [])
            let base = CVPixelBufferGetBaseAddress(pixel)!.assumingMemoryBound(to: UInt32.self)
            let count = CVPixelBufferGetBytesPerRow(pixel) * height / 4
            var seed = UInt32(frame + 1)
            for i in 0..<count {
                seed = seed &* 1_664_525 &+ 1_013_904_223
                base[i] = seed | 0xFF
            }
            CVPixelBufferUnlockBaseAddress(pixel, [])
            XCTAssertTrue(adaptor.append(pixel, withPresentationTime: CMTime(value: Int64(frame), timescale: 15)))
        }
        input.markAsFinished()
        await withCheckedContinuation { continuation in writer.finishWriting { continuation.resume() } }
        XCTAssertEqual(writer.status, .completed, writer.error?.localizedDescription ?? "")
        return url
    }
}
