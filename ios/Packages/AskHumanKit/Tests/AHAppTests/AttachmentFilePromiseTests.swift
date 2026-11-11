import XCTest
import UniformTypeIdentifiers
import AHProtocol
@testable import AHCore

@MainActor
final class AttachmentFilePromiseTests: XCTestCase {
    private let asset = AssetRef(assetId: "promisedassetpromisedasset", name: "report.csv", size: 12, mime: "text/csv", isImage: false, tooLarge: false)

    private func source(_ bytes: Data) throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("opaque-cache-id")
        try bytes.write(to: url)
        return url
    }

    private func receive(_ provider: NSItemProvider, type: String) async throws -> (String, Data) {
        try await withCheckedThrowingContinuation { continuation in
            provider.loadFileRepresentation(forTypeIdentifier: type) { url, error in
                do {
                    if let error { throw error }
                    let url = try XCTUnwrap(url)
                    continuation.resume(returning: (url.lastPathComponent, try Data(contentsOf: url)))
                } catch { continuation.resume(throwing: error) }
            }
        }
    }

    func testTheProviderIsLazyAndDeliversTheNamedOriginalFile() async throws {
        let bytes = Data("a,b\n1,2\n".utf8)
        let source = try source(bytes)
        let calls = Calls()
        let promise = AttachmentFilePromise(asset: asset) { progress in
            calls.hit()
            progress(0.5)
            return source
        }
        let provider = promise.provider()
        XCTAssertEqual(calls.count, 0, "a lift without a receiver must not start a download")
        XCTAssertEqual(provider.suggestedName, "report.csv")
        XCTAssertTrue(provider.hasItemConformingToTypeIdentifier(UTType.commaSeparatedText.identifier))
        let result = try await receive(provider, type: promise.typeIdentifier)
        XCTAssertEqual(result.0, "report.csv")
        XCTAssertEqual(result.1, bytes)
        XCTAssertEqual(calls.count, 1)
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path), "export copies rather than moves the cache")
    }

    func testAnImageAdvertisesItsOriginalFormat() {
        var image = asset
        image.name = "original.png"
        image.mime = "image/png"
        image.isImage = true
        let promise = AttachmentFilePromise(asset: image) { _ in throw BackendError.unavailable("not requested") }
        XCTAssertEqual(promise.typeIdentifier, UTType.png.identifier)
        XCTAssertFalse(promise.provider().hasItemConformingToTypeIdentifier(UTType.url.identifier))
    }

    func testAGenericMimeDoesNotHideAKnownFileExtension() {
        var file = asset
        file.name = "document.pdf"
        file.mime = "application/octet-stream"
        let promise = AttachmentFilePromise(asset: file) { _ in throw BackendError.macOffline }
        XCTAssertEqual(promise.typeIdentifier, UTType.pdf.identifier)
    }

    func testAnAppWithoutTheSpecificTypeCanRequestTheOriginalAsAFile() async throws {
        let bytes = Data("original csv bytes".utf8)
        let source = try source(bytes)
        let promise = AttachmentFilePromise(asset: asset) { _ in source }
        let result = try await receive(promise.provider(), type: UTType.data.identifier)
        XCTAssertEqual(result.0, "report.csv")
        XCTAssertEqual(result.1, bytes)
    }

    func testDownloadErrorsDoNotTurnIntoEmptyFiles() async {
        let promise = AttachmentFilePromise(asset: asset) { _ in throw BackendError.macOffline }
        do {
            _ = try await receive(promise.provider(), type: promise.typeIdentifier)
            XCTFail("expected the file request to fail")
        } catch { XCTAssertFalse(error.localizedDescription.isEmpty) }
    }

    func testTheReceivingAppCanCancelAnAsyncDownload() async throws {
        let started = expectation(description: "the receiver requests the file")
        let finished = expectation(description: "the cancelled request completes with an error")
        let promise = AttachmentFilePromise(asset: asset) { _ in
            started.fulfill()
            try await Task.sleep(for: .seconds(30))
            throw BackendError.unavailable("must have cancelled")
        }
        let provider = promise.provider()
        let progress = provider.loadFileRepresentation(forTypeIdentifier: promise.typeIdentifier) { url, error in
            XCTAssertNil(url)
            XCTAssertNotNil(error)
            finished.fulfill()
        }
        await fulfillment(of: [started], timeout: 5)
        progress.cancel()
        await fulfillment(of: [finished], timeout: 5)
    }

    func testAPathLikeNameCannotEscapeTheExportDirectory() async throws {
        var file = asset
        file.name = "../../outside.csv"
        let source = try source(Data("original".utf8))
        let promise = AttachmentFilePromise(asset: file) { _ in source }
        let result = try await receive(promise.provider(), type: promise.typeIdentifier)
        XCTAssertFalse(result.0.contains("/"))
        XCTAssertEqual(result.1, Data("original".utf8))
    }

    private final class Calls: @unchecked Sendable {
        private let lock = NSLock()
        private var value = 0
        var count: Int { lock.withLock { value } }
        func hit() { lock.withLock { value += 1 } }
    }
}
