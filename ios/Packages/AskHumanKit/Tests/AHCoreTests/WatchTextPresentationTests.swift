import XCTest
@testable import AHCore
import AHProtocol

final class WatchTextPresentationTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1000)
    private func at(_ seconds: TimeInterval) -> Date { start.addingTimeInterval(seconds) }
    private func text(_ value: String, kind: String = "thinking") -> WatchLatestText {
        WatchLatestText(id: value, revision: value, kind: kind, text: value)
    }

    func testDenseUpdatesUseAFixedDeadline() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.receive(text("second"), now: at(1))
        view.receive(text("latest"), now: at(9))
        XCTAssertEqual(view.displayed?.text, "first")
        XCTAssertEqual(view.deadline, at(10))
        view.tick(now: at(10))
        XCTAssertEqual(view.displayed?.text, "latest")
        XCTAssertNil(view.deadline)
    }

    func testAnOpenedTextIsStableAndClosingItShowsTheLatestProse() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.toggleExpanded(now: start)
        view.receive(text("second"), now: at(20))
        view.tick(now: at(30))
        XCTAssertEqual(view.displayed?.text, "first")
        XCTAssertTrue(view.hasNewContent)
        XCTAssertNil(view.deadline, "an opened text holds without an end")
        view.updateNow(now: at(30))
        XCTAssertEqual(view.displayed?.text, "second")
        XCTAssertTrue(view.reading, "still open")
        view.receive(text("prose", kind: "assistant"), now: at(40))
        view.toggleExpanded(now: at(41))
        XCTAssertEqual(view.displayed?.kind, "assistant")
        XCTAssertFalse(view.reading)
    }

    func testAFingerOnTheTextHoldsItAndTheLeaseOutlastsTheFinger() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.touchBegan()
        view.receive(text("next"), now: at(15))
        XCTAssertEqual(view.displayed?.text, "first", "a finger on the text holds it")
        XCTAssertTrue(view.reading)
        XCTAssertTrue(view.hasNewContent)
        XCTAssertNil(view.deadline, "there is no end to wait for while the finger is on it")
        view.tick(now: at(60))
        XCTAssertEqual(view.displayed?.text, "first")

        view.touchEnded(now: at(70))
        XCTAssertTrue(view.reading, "the lease holds it for a moment after the finger left")
        XCTAssertEqual(view.deadline, at(70 + WatchTextPresentation.lease))
        view.tick(now: at(72.9))
        XCTAssertEqual(view.displayed?.text, "first")
        view.tick(now: at(73))
        XCTAssertEqual(view.displayed?.text, "next", "the lease is over: the new text replaces it by itself")
        XCTAssertFalse(view.reading)
        XCTAssertFalse(view.hasNewContent)
    }

    func testAFingerThatComesBackRenewsTheLease() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.touchBegan()
        view.touchEnded(now: at(20))
        view.receive(text("next"), now: at(21))
        XCTAssertEqual(view.deadline, at(23))

        view.touchBegan()
        XCTAssertNil(view.deadline, "the first lease is gone with the finger's return")
        view.tick(now: at(24))
        XCTAssertEqual(view.displayed?.text, "first")
        view.touchEnded(now: at(25))
        XCTAssertEqual(view.deadline, at(28))
        view.tick(now: at(27.9))
        XCTAssertEqual(view.displayed?.text, "first")
        view.tick(now: at(28))
        XCTAssertEqual(view.displayed?.text, "next")
    }

    func testTheLeaseNeverShortensTheReplacementInterval() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.touchBegan()
        view.touchEnded(now: at(1))
        view.receive(text("next"), now: at(2))
        XCTAssertEqual(view.deadline, at(WatchTextPresentation.interval), "the lease ends at 4, the interval at 10")
        view.tick(now: at(5))
        XCTAssertEqual(view.displayed?.text, "first")
        XCTAssertFalse(view.reading, "the lease is over, so only the interval is waited for")
        XCTAssertTrue(view.hasNewContent)
        view.tick(now: at(10))
        XCTAssertEqual(view.displayed?.text, "next")
    }

    func testTextThatArrivesAfterTheLeaseIsShownAtOnce() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.touchBegan()
        view.touchEnded(now: at(20))
        view.receive(text("next"), now: at(30))
        XCTAssertEqual(view.displayed?.text, "next", "nothing is held any more, and the interval has passed")
        XCTAssertFalse(view.reading)
    }

    func testAskingForTheNewestEndsTheLeaseAndClosingEndsItToo() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.touchBegan()
        view.touchEnded(now: at(20))
        view.receive(text("next"), now: at(21))
        view.updateNow(now: at(22))
        XCTAssertEqual(view.displayed?.text, "next")
        XCTAssertFalse(view.reading)

        view.touchBegan()
        view.touchEnded(now: at(30))
        view.toggleExpanded(now: at(31))
        view.toggleExpanded(now: at(32))
        XCTAssertFalse(view.reading, "closing a text that was opened leaves nothing held")
    }

    func testAFingerThatNeverBeganDoesNotStartALease() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.touchEnded(now: at(20))
        XCTAssertFalse(view.reading)
        view.receive(text("next"), now: at(21))
        XCTAssertEqual(view.displayed?.text, "next")
    }

    func testRepeatedFramesDoNotResetTheWindow() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.touchBegan()
        view.receive(text("next"), now: at(20))
        XCTAssertEqual(view.displayed?.text, "first")
        view.updateNow(now: at(21))
        view.touchEnded(now: at(21))
        view.receive(text("next"), now: at(25))
        XCTAssertFalse(view.hasNewContent, "the text that is shown arriving again is not new")
        view.tick(now: at(30))
        XCTAssertFalse(view.reading)
        view.receive(text("last"), now: at(30))
        XCTAssertEqual(view.deadline, at(31))
        view.receive(nil, now: at(30))
        view.tick(now: at(31))
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
