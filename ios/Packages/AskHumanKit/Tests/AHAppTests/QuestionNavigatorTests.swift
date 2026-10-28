import CoreGraphics
import XCTest
@testable import AHCore

/// Where "Next" leads follows the position on screen, not what was looked at before.
final class QuestionNavigatorTests: XCTestCase {
    typealias Viewport = QuestionNavigator.Viewport
    typealias Target = QuestionNavigator.Target

    /// A phone screen: the visible area runs from 100 to 700 (600 high).
    private func viewport(_ cards: [CGRect], top: CGFloat = 100, bottom: CGFloat = 700) -> Viewport {
        var map: [Int: CGRect] = [:]
        for (index, frame) in cards.enumerated() { map[index] = frame }
        return Viewport(cards: map, top: top, bottom: bottom)
    }

    private func card(_ minY: CGFloat, _ height: CGFloat) -> CGRect {
        CGRect(x: 16, y: minY, width: 358, height: height)
    }

    /// Three cards of 330 with 16 between them, scrolled so that the first one starts at `first`.
    private func three(first: CGFloat) -> Viewport {
        viewport([card(first, 330), card(first + 346, 330), card(first + 692, 330)])
    }

    func testNextLeadsToTheFirstQuestionWhoseBottomIsNotOnScreen() {
        // Q1 is on screen (110–440), Q2 sticks out at the bottom (456–786), Q3 is below.
        XCTAssertEqual(QuestionNavigator.next(in: three(first: 110), count: 3), Target(question: 1, anchor: .bottom))
    }

    func testAQuestionSeenButNotAnsweredIsNotSkippedAfterScrollingBack() {
        // The person went down to the end and came back: the screen shows the top again, as when the
        // form opened. "Next" leads to the question after the one on screen, never past the ones that
        // were looked at on the way.
        XCTAssertEqual(QuestionNavigator.next(in: three(first: 110), count: 3)?.question, 1)
        // Scrolled down by 346: Q1 is above the screen, Q2 is all on screen, Q3 is cut off.
        XCTAssertEqual(QuestionNavigator.next(in: three(first: -236), count: 3)?.question, 2)
    }

    func testALongMessageThatHidesTheFirstQuestionLeadsToTheFirstQuestion() {
        XCTAssertEqual(QuestionNavigator.next(in: three(first: 900), count: 3), Target(question: 0, anchor: .bottom))
    }

    func testAQuestionCutOffAtTheBottomIsTheNextOne() {
        let screen = viewport([card(110, 520), card(646, 300)])
        XCTAssertEqual(QuestionNavigator.next(in: screen, count: 2)?.question, 1)
    }

    func testNothingIsLeftOnceTheLastQuestionIsOnScreen() {
        XCTAssertNil(QuestionNavigator.next(in: three(first: -500), count: 3))
        XCTAssertNil(QuestionNavigator.next(in: viewport([card(110, 100), card(226, 200)]), count: 2), "a form that fits")
    }

    func testAQuestionTallerThanTheScreenIsShownFromItsTopThenFromItsBottom() {
        // Q2 is 900 high: more than the 600 of the screen. Q1 and Q3 are 200 high, 16 apart from it.
        func form(scrolledTo q2Top: CGFloat) -> Viewport {
            viewport([card(q2Top - 216, 200), card(q2Top, 900), card(q2Top + 916, 200)])
        }
        // Its top is below the top edge: show the top first.
        XCTAssertEqual(QuestionNavigator.next(in: form(scrolledTo: 326), count: 3), Target(question: 1, anchor: .top))
        // Its top is at the edge: the bottom is the next stop.
        XCTAssertEqual(QuestionNavigator.next(in: form(scrolledTo: 100), count: 3), Target(question: 1, anchor: .bottom))
        // Past its top: still its bottom, not the top again.
        XCTAssertEqual(QuestionNavigator.next(in: form(scrolledTo: -100), count: 3), Target(question: 1, anchor: .bottom))
        // Once its bottom is on screen it is passed, and so is the last one here.
        XCTAssertNil(QuestionNavigator.next(in: form(scrolledTo: -500), count: 3))
    }

    func testAQuestionThatJustFitsIsLinedUpWithTheBottomWithAGap() {
        // 584 high leaves the 16 of gap in the 600 of the screen; one point more does not fit.
        let fits = viewport([card(110, 100), card(226, 584)])
        XCTAssertEqual(QuestionNavigator.next(in: fits, count: 2), Target(question: 1, anchor: .bottom))
        let tooTall = viewport([card(110, 100), card(226, 585)])
        XCTAssertEqual(QuestionNavigator.next(in: tooTall, count: 2), Target(question: 1, anchor: .top))
    }

    func testMissingFramesAreLeftOut() {
        var screen = three(first: 110)
        screen.cards[1] = nil
        XCTAssertEqual(QuestionNavigator.next(in: screen, count: 3)?.question, 2, "an unmeasured card cannot be the target")
        XCTAssertNil(QuestionNavigator.next(in: Viewport(cards: [:], top: 100, bottom: 700), count: 3), "nothing is laid out yet")
    }

    func testASingleQuestionHasNoNext() {
        XCTAssertNil(QuestionNavigator.next(in: viewport([card(110, 250)]), count: 1))
    }

    // MARK: revealing a given question

    func testRevealLeavesAQuestionThatIsAllOnScreenAlone() {
        let screen = viewport([card(110, 250), card(376, 250)])
        XCTAssertNil(QuestionNavigator.reveal(1, in: screen))
    }

    func testRevealShowsAQuestionAboveTheScreenFromItsTop() {
        let screen = viewport([card(-300, 250), card(-34, 250), card(232, 250)])
        XCTAssertEqual(QuestionNavigator.reveal(0, in: screen), Target(question: 0, anchor: .top))
        XCTAssertEqual(QuestionNavigator.reveal(1, in: screen), Target(question: 1, anchor: .top), "its top is cut off")
    }

    func testRevealShowsAQuestionBelowTheScreenFromItsBottomWhenItFits() {
        let screen = viewport([card(110, 250), card(900, 250)])
        XCTAssertEqual(QuestionNavigator.reveal(1, in: screen), Target(question: 1, anchor: .bottom))
        let tall = viewport([card(110, 250), card(900, 800)])
        XCTAssertEqual(QuestionNavigator.reveal(1, in: tall), Target(question: 1, anchor: .top))
        XCTAssertNil(QuestionNavigator.reveal(5, in: screen), "not there at all")
    }
}
