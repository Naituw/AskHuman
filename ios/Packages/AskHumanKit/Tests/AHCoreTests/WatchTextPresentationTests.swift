import XCTest
@testable import AHCore
import AHProtocol

final class WatchTextPresentationTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1000)
    private func text(_ value: String, kind: String = "thinking") -> WatchLatestText {
        WatchLatestText(id: value, revision: value, kind: kind, text: value)
    }

    func testDenseUpdatesUseAFixedDeadline() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.receive(text("second"), now: start.addingTimeInterval(1))
        view.receive(text("latest"), now: start.addingTimeInterval(9))
        XCTAssertEqual(view.displayed?.text, "first")
        XCTAssertEqual(view.deadline, start.addingTimeInterval(10))
        view.tick(now: start.addingTimeInterval(10))
        XCTAssertEqual(view.displayed?.text, "latest")
        XCTAssertNil(view.deadline)
    }

    func testReadingIsStableAndCollapseShowsTheLatestProse() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.toggleExpanded(now: start)
        view.receive(text("second"), now: start.addingTimeInterval(20))
        view.tick(now: start.addingTimeInterval(30))
        XCTAssertEqual(view.displayed?.text, "first")
        XCTAssertTrue(view.hasNewContent)
        XCTAssertNil(view.deadline)
        view.updateNow(now: start.addingTimeInterval(30))
        XCTAssertEqual(view.displayed?.text, "second")
        XCTAssertTrue(view.reading)
        view.receive(text("prose", kind: "assistant"), now: start.addingTimeInterval(40))
        view.toggleExpanded(now: start.addingTimeInterval(41))
        XCTAssertEqual(view.displayed?.kind, "assistant")
        XCTAssertFalse(view.reading)
    }

    func testSelectionLocksAndRepeatedFramesDoNotResetTheWindow() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.lockReading()
        view.receive(text("next"), now: start.addingTimeInterval(20))
        XCTAssertEqual(view.displayed?.text, "first")
        view.updateNow(now: start.addingTimeInterval(21))
        XCTAssertFalse(view.reading)
        view.receive(text("next"), now: start.addingTimeInterval(25))
        view.receive(text("last"), now: start.addingTimeInterval(30))
        XCTAssertEqual(view.deadline, start.addingTimeInterval(31))
        view.receive(nil, now: start.addingTimeInterval(30))
        view.tick(now: start.addingTimeInterval(31))
        XCTAssertEqual(view.displayed?.text, "last")
    }

    func testFramesDecodeNewAndLegacyTextLeniently() throws {
        let legacy = try XCTUnwrap(WatchFrameData(json: .object(["text": .string("legacy")])))
        XCTAssertNil(legacy.latestText)
        XCTAssertFalse(legacy.supportsLatestText)
        let buffering = try XCTUnwrap(WatchFrameData(json: .object(["text": .string("growing"), "latestText": .null])))
        XCTAssertTrue(buffering.supportsLatestText)
        XCTAssertNil(buffering.latestText)
        let frame = try XCTUnwrap(WatchFrameData(json: .object([
            "text": .string("old prose"),
            "latestText": .object(["id": .string("one"), "revision": .string("r1"),
                "kind": .string("thinking"), "text": .string("thought")]),
        ])))
        XCTAssertEqual(frame.latestText?.text, "thought")
        XCTAssertEqual(frame.text, "old prose")
    }
}
