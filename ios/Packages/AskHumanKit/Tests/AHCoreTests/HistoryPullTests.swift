import XCTest
@testable import AHCore

/// The pull-down for a session's history (docs/specs/ios-session-history.md, S18), without a screen: what a finger does to
/// the page is fed in, what the header shows and whether to load comes out.
final class HistoryPullTests: XCTestCase {
    private let threshold = HistoryPull.defaultThreshold
    private let dead = HistoryPull.deadZone

    private func drag(_ pull: inout HistoryPull, to overscroll: CGFloat, enabled: Bool = true) -> HistoryPull.Outcome {
        pull.update(overscroll: overscroll, dragging: true, enabled: enabled)
    }

    func testNothingIsShownWhileThePageIsAtItsTopOrBarelyPulled() {
        var pull = HistoryPull()
        XCTAssertEqual(drag(&pull, to: 0), .nothing)
        XCTAssertEqual(pull.phase, .idle)
        XCTAssertEqual(pull.opacity, 0)
        XCTAssertEqual(drag(&pull, to: dead), .nothing, "the dead zone is not left until it is passed")
        XCTAssertEqual(pull.phase, .idle)
        XCTAssertEqual(pull.opacity, 0)
        _ = drag(&pull, to: -30)
        XCTAssertEqual(pull.phase, .idle, "a page scrolled into its content is not pulled")
    }

    func testTheHeaderFadesInAndTheIconTurnsBackwardsWithTheDistance() {
        var pull = HistoryPull()
        _ = drag(&pull, to: dead + 1)
        XCTAssertEqual(pull.phase, .pulling)
        XCTAssertGreaterThan(pull.opacity, 0)
        XCTAssertLessThan(pull.opacity, 0.2)
        _ = drag(&pull, to: HistoryPull.fullSight)
        XCTAssertEqual(pull.opacity, 1, accuracy: 0.0001)
        let halfway = (dead + threshold) / 2
        _ = drag(&pull, to: halfway)
        XCTAssertEqual(pull.progress, 0.5, accuracy: 0.0001)
        XCTAssertEqual(pull.turn, -.pi, accuracy: 0.0001, "half a turn, counter-clockwise")
        _ = drag(&pull, to: threshold - 0.5)
        XCTAssertLessThan(pull.progress, 1)
        XCTAssertEqual(pull.phase, .pulling)
        _ = drag(&pull, to: threshold + 80)
        XCTAssertEqual(pull.progress, 1, "the turn stops at a full one however far the pull goes")
        XCTAssertEqual(pull.turn, -2 * .pi, accuracy: 0.0001)
    }

    func testCrossingTheThresholdArmsAndTapsOnce() {
        var pull = HistoryPull()
        XCTAssertEqual(drag(&pull, to: threshold - 1), .nothing)
        XCTAssertEqual(drag(&pull, to: threshold), .armed)
        XCTAssertTrue(pull.isArmed)
        XCTAssertEqual(drag(&pull, to: threshold + 20), .nothing, "no second tap while it stays armed")
        XCTAssertEqual(drag(&pull, to: threshold + 60), .nothing)
        XCTAssertTrue(pull.isArmed)
    }

    func testAFingerRestingOnTheThresholdDoesNotTapAgainAndAgain() {
        var pull = HistoryPull()
        XCTAssertEqual(drag(&pull, to: threshold + 1), .armed)
        // Wobbling by a couple of points around the line: still armed, no tap.
        XCTAssertEqual(drag(&pull, to: threshold - 2), .nothing)
        XCTAssertTrue(pull.isArmed)
        XCTAssertEqual(drag(&pull, to: threshold + 2), .nothing)
        XCTAssertTrue(pull.isArmed)
    }

    func testComingBackUnderTheThresholdDisarmsAndGoingOverAgainTapsAgain() {
        var pull = HistoryPull()
        XCTAssertEqual(drag(&pull, to: threshold + 10), .armed)
        XCTAssertEqual(drag(&pull, to: threshold - HistoryPull.hysteresis - 1), .nothing)
        XCTAssertEqual(pull.phase, .pulling, "back under: letting go now would load nothing")
        XCTAssertEqual(drag(&pull, to: threshold + 1), .armed, "over the line again: another tap")
    }

    func testLettingGoWhileArmedLoadsAndOnlyOnce() {
        var pull = HistoryPull()
        _ = drag(&pull, to: threshold + 30)
        XCTAssertTrue(pull.release())
        XCTAssertEqual(pull.phase, .idle)
        XCTAssertFalse(pull.release(), "the same lift is not a second load")
    }

    func testLettingGoShortOfTheThresholdLoadsNothing() {
        var pull = HistoryPull()
        _ = drag(&pull, to: threshold - 10)
        XCTAssertEqual(pull.phase, .pulling)
        XCTAssertFalse(pull.release())
        XCTAssertEqual(pull.phase, .idle)
        XCTAssertEqual(pull.opacity, 0, "the header goes with the pull")
    }

    func testTheLiftIsAlsoSeenFromTheFingerStoppingToDragAndThenReleaseFindsNothingLeftToDecide() {
        var pull = HistoryPull()
        _ = drag(&pull, to: threshold + 25)
        // The page reports itself again with the finger gone, before the pan gesture's own end arrived.
        XCTAssertEqual(pull.update(overscroll: threshold + 20, dragging: false, enabled: true), .load)
        XCTAssertFalse(pull.release(), "the gesture's end comes second and has nothing to load")
        XCTAssertEqual(pull.phase, .idle)
    }

    func testABounceOrAFlickPastTheTopWithoutAFingerIsNoPull() {
        var pull = HistoryPull()
        XCTAssertEqual(pull.update(overscroll: threshold + 40, dragging: false, enabled: true), .nothing)
        XCTAssertEqual(pull.phase, .idle)
        XCTAssertEqual(pull.opacity, 0)
        XCTAssertFalse(pull.release())
    }

    func testAPageThatCannotBePulledForHistoryNeverShowsOrLoads() {
        var pull = HistoryPull()
        XCTAssertEqual(drag(&pull, to: threshold + 50, enabled: false), .nothing)
        XCTAssertEqual(pull.phase, .idle)
        XCTAssertEqual(pull.opacity, 0)
        XCTAssertFalse(pull.release())
    }

    func testTheEntryGoingAwayMidPullDropsThePull() {
        var pull = HistoryPull()
        _ = drag(&pull, to: threshold + 10)
        XCTAssertTrue(pull.isArmed)
        XCTAssertEqual(drag(&pull, to: threshold + 12, enabled: false), .nothing, "a load started elsewhere, or the start was reached")
        XCTAssertEqual(pull.phase, .idle)
        XCTAssertFalse(pull.release())
    }

    func testTheThresholdMayBeTallerWhereTheRoomForTheLoadingLineIs() {
        var pull = HistoryPull(threshold: 90)
        XCTAssertEqual(drag(&pull, to: 80), .nothing, "past the standard threshold, short of this page's")
        XCTAssertLessThan(pull.progress, 1)
        XCTAssertEqual(drag(&pull, to: 90), .armed)
        XCTAssertEqual(pull.progress, 1, accuracy: 0.0001)
    }

    func testCancelForgetsEverything() {
        var pull = HistoryPull()
        _ = drag(&pull, to: threshold + 10)
        pull.cancel()
        XCTAssertEqual(pull.phase, .idle)
        XCTAssertEqual(pull.overscroll, 0)
        XCTAssertFalse(pull.release())
    }
}
