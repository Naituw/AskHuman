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

    /// Opening a text only makes its window taller (2026-11-15; it used to hold the words until it was closed, which made
    /// "New content" the only way to see what the Agent said next). The words are held for the moment after the tap, as
    /// they are after a finger, and then the next text replaces them by itself like it does a closed one.
    func testAnOpenedTextIsReplacedByItselfLikeAClosedOne() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.toggleExpanded(now: at(11))
        view.receive(text("next"), now: at(12))
        XCTAssertEqual(view.displayed?.text, "first", "the moment after opening it is held")
        XCTAssertEqual(view.deadline, at(11 + WatchTextPresentation.lease))
        view.tick(now: at(13.9))
        XCTAssertEqual(view.displayed?.text, "first")
        view.tick(now: at(14))
        XCTAssertEqual(view.displayed?.text, "next", "nothing holds it any more: it is replaced by itself")
        XCTAssertTrue(view.expanded, "it stays open, whatever the words are")
        XCTAssertFalse(view.reading)
        XCTAssertFalse(view.hasNewContent)
    }

    func testAnOpenedTextThatNothingTouchesIsWaitedForNoLongerThanTheInterval() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.toggleExpanded(now: at(1))
        view.receive(text("next"), now: at(2))
        XCTAssertEqual(view.deadline, at(WatchTextPresentation.interval), "the lease ends at 4, the interval at 10")
        XCTAssertEqual(view.wake, at(1 + WatchTextPresentation.lease), "the view wakes when the lease ends, not only when the text is due")
        view.tick(now: at(4))
        XCTAssertFalse(view.reading, "the lease is over, so only the interval is waited for")
        XCTAssertTrue(view.hasNewContent, "what is pending still waits for the interval")
        XCTAssertEqual(view.wake, at(WatchTextPresentation.interval))
        view.tick(now: at(9.9))
        XCTAssertEqual(view.displayed?.text, "first")
        view.tick(now: at(10))
        XCTAssertEqual(view.displayed?.text, "next")
        XCTAssertTrue(view.expanded)
    }

    /// The view ticks at `wake`. A lease that ends while the text is still waiting for the interval has to be one of those
    /// moments: with only the moment the text is due, the lease went on being counted (and "New content" stayed on the
    /// screen) until the text was replaced, up to ten seconds after it was over.
    func testTheViewWakesWhenALeaseEndsAndWhenTheTextIsDue() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        XCTAssertNil(view.wake, "nothing waits for time to pass")
        view.touchBegan()
        view.receive(text("next"), now: at(2))
        XCTAssertNil(view.wake, "a finger on the text has no end to wake for")
        view.touchEnded(now: at(5))
        XCTAssertEqual(view.wake, at(5 + WatchTextPresentation.lease), "the lease ends (8) before the text is due (10)")
        view.tick(now: at(8))
        XCTAssertFalse(view.reading)
        XCTAssertEqual(view.wake, at(WatchTextPresentation.interval))
        view.tick(now: at(10))
        XCTAssertEqual(view.displayed?.text, "next")
        XCTAssertNil(view.wake)

        // Nothing pending: the end of the lease is still a moment to wake at, and then there is none.
        var idle = WatchTextPresentation()
        idle.receive(text("first"), now: start)
        idle.touchBegan()
        idle.touchEnded(now: at(5))
        XCTAssertEqual(idle.wake, at(8))
        idle.tick(now: at(8))
        XCTAssertNil(idle.wake)
        XCTAssertFalse(idle.reading)
    }

    func testAFingerOnAnOpenedTextHoldsItAsOnAClosedOne() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.toggleExpanded(now: at(1))
        view.touchBegan()
        view.receive(text("next"), now: at(30))
        XCTAssertEqual(view.displayed?.text, "first")
        XCTAssertNil(view.deadline, "there is no end to wait for while the finger is on it")
        view.tick(now: at(60))
        XCTAssertEqual(view.displayed?.text, "first")
        view.touchEnded(now: at(70))
        view.tick(now: at(73))
        XCTAssertEqual(view.displayed?.text, "next")
        XCTAssertTrue(view.expanded)
    }

    func testClosingATextShowsTheLatestAtOnce() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.toggleExpanded(now: at(1))
        view.receive(text("next"), now: at(2))
        XCTAssertEqual(view.displayed?.text, "first", "held by the moment after opening, and by the interval")
        view.toggleExpanded(now: at(3))
        XCTAssertEqual(view.displayed?.text, "next", "closing goes back to the overview, which shows the newest")
        XCTAssertFalse(view.expanded)
        XCTAssertFalse(view.reading)
        view.receive(text("prose", kind: "assistant"), now: at(5))
        XCTAssertEqual(view.deadline, at(3 + WatchTextPresentation.interval), "the interval starts again from the closing")
    }

    func testAskingForTheNewestKeepsAnOpenedTextOpen() {
        var view = WatchTextPresentation()
        view.receive(text("first"), now: start)
        view.touchBegan()
        view.toggleExpanded(now: at(1))
        view.receive(text("next"), now: at(2))
        XCTAssertTrue(view.hasNewContent)
        view.updateNow(now: at(3))
        XCTAssertEqual(view.displayed?.text, "next")
        XCTAssertTrue(view.expanded, "the window stays as tall as it was")
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
