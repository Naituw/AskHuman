import XCTest
import ImageIO
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

/// The pictures of the history against the mock Macs (docs/specs/ios-history-pictures.md): what a thread's pages carry, how
/// the History tab's page gets the exchange of a finished request, and what it is when the Mac cannot say.
@MainActor
final class HistoryPicturesStoreTests: XCTestCase {
    private var backend: MockBackend!
    private var app: AppStore!

    private func make(_ scenario: MockBackend.Scenario = .historyPictures) async {
        backend = MockBackend(scenario: scenario, latency: 0)
        let defaults = UserDefaults(suiteName: "ah-historypictures-test-" + UUID().uuidString)!
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        await app.start()
        await eventually("the Macs, the inbox and the history") {
            self.app.state.foreground && self.app.state.macs.count == 2 && !self.app.state.inbox.isEmpty && !self.app.history.isEmpty
        }
    }

    private func entry(_ requestId: String) throws -> HistoryEntry {
        try XCTUnwrap(app.history.first { $0.request.id == requestId }, "the History tab has \(requestId)")
    }

    // MARK: a thread

    func testThePagesOfAThreadCarryTheirPictures() async throws {
        await make()
        let item = try XCTUnwrap(app.state.inbox.first { $0.requestId == MockData.picturesRequestId })
        let thread = try XCTUnwrap(app.makeSessionThread(for: item))
        await thread.loadMore()
        XCTAssertEqual(thread.items.count, 5, "the whole pictures session is one page")
        let byId = Dictionary(uniqueKeysWithValues: thread.items.map { ($0.id, $0) })
        let seven = try XCTUnwrap(byId["ex-pics-05"])
        XCTAssertEqual(seven.pictures.count, 7)
        XCTAssertEqual(seven.attachments, 8, "the count still has the file that is not a picture")
        XCTAssertTrue(seven.pictures.allSatisfy { $0.thumb != nil && !$0.gone })
        XCTAssertEqual(byId["ex-pics-03"]?.pictures.map(\.gone), [false, true])
        XCTAssertEqual(byId["ex-pics-02"]?.pictures.map(\.gone), [true, true], "all gone: places held, nothing to show")
        let reply = try XCTUnwrap(byId["ex-pics-01"]?.qs.first)
        XCTAssertEqual(reply.pictures.count, 2)
        XCTAssertEqual(reply.images, 2)
    }

    func testTheOlderScenarioHasNoPicturesInItsPages() async throws {
        backend = MockBackend(scenario: .sessionHistory, latency: 0)
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: UserDefaults(suiteName: "ah-historypictures-old-" + UUID().uuidString)!)
        await app.start()
        await eventually("the inbox") { !self.app.state.inbox.isEmpty }
        let item = try XCTUnwrap(app.state.inbox.first { $0.requestId == "req-history-long" })
        let thread = try XCTUnwrap(app.makeSessionThread(for: item))
        await thread.loadMore()
        XCTAssertFalse(thread.items.isEmpty)
        XCTAssertTrue(thread.items.allSatisfy { $0.pictures.isEmpty && $0.qs.allSatisfy { $0.pictures.isEmpty } })
    }

    // MARK: the History tab

    func testAFinishedRequestGetsItsExchangeWithThePictures() async throws {
        await make()
        let picked = try entry(MockData.picturesHistoryRequestId)
        let exchange = await app.historyExchange(for: picked)
        XCTAssertEqual(exchange?.id, MockData.picturesHistoryRequestId)
        XCTAssertEqual(exchange?.pictures.count, 7)
        let calls = await backend.sessionExchangeCalls
        XCTAssertEqual(calls, [MockBackend.SessionExchangeCall(macId: MockData.homeMacId, id: MockData.picturesHistoryRequestId)])
    }

    func testARecordTheMacDoesNotHaveIsNothingToShow() async throws {
        await make()
        let forgotten = try entry("req-old-1")
        let exchange = await app.historyExchange(for: forgotten)
        XCTAssertNil(exchange)
        let calls = await backend.sessionExchangeCalls
        XCTAssertEqual(calls.count, 1, "the Mac was asked, and had no such record")
    }

    func testAMacThatDoesNotDescribePicturesIsNotAsked() async throws {
        await make()
        let elsewhere = try entry("req-old-2")
        XCTAssertEqual(elsewhere.macId, MockData.officeMacId)
        let exchange = await app.historyExchange(for: elsewhere)
        XCTAssertNil(exchange)
        let calls = await backend.sessionExchangeCalls
        XCTAssertTrue(calls.isEmpty, "an older Mac is not sent what it cannot read")
    }

    func testAnOfflineMacLeavesThePageAsItWas() async throws {
        await make()
        let picked = try entry(MockData.picturesHistoryRequestId)
        await backend.setUnreachable([MockData.homeMacId])
        let exchange = await app.historyExchange(for: picked)
        XCTAssertNil(exchange, "no error to show: the page is whole without the pictures")
    }

    func testTheOlderScenarioNeverAsksForAnExchange() async throws {
        backend = MockBackend(scenario: .sessionHistory, latency: 0)
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: UserDefaults(suiteName: "ah-historypictures-none-" + UUID().uuidString)!)
        await app.start()
        await eventually("the history") { !self.app.history.isEmpty }
        let exchange = await app.historyExchange(for: try XCTUnwrap(app.history.first))
        XCTAssertNil(exchange)
        let calls = await backend.sessionExchangeCalls
        XCTAssertTrue(calls.isEmpty)
    }

    // MARK: the originals

    func testAHistoryPictureDownloadsAsTheImageItStandsFor() async throws {
        await make()
        let picture = MockData.historyPicture(3, name: "tall.png", width: 800, height: 1200, hue: 0.33)
        let url = try await backend.download(macId: MockData.homeMacId, asset: picture) { _ in }
        let data = try Data(contentsOf: url)
        XCTAssertEqual(Array(data.prefix(4)), [0x89, 0x50, 0x4E, 0x47], "a PNG")
        let source = try XCTUnwrap(CGImageSourceCreateWithData(data as CFData, nil))
        let properties = try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        XCTAssertEqual(properties[kCGImagePropertyPixelWidth] as? Int, 800)
        XCTAssertEqual(properties[kCGImagePropertyPixelHeight] as? Int, 1200)
    }

    func testTheThumbnailsOfThePicturesAreSmallJPEGs() async throws {
        let picture = MockData.historyPicture(1, width: 1200, height: 800)
        let data = try XCTUnwrap(picture.thumb.flatMap(Bytes.b64uDecode))
        XCTAssertEqual(Array(data.prefix(2)), [0xFF, 0xD8], "a JPEG")
        XCTAssertLessThanOrEqual(data.count, Wire.historyThumbMaxBytes * 2, "a thumbnail is small")
        XCTAssertNil(MockData.historyPicture(2, gone: true).thumb)
        XCTAssertTrue(MockData.historyPicture(2, gone: true).gone)
    }
}
