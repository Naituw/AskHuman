import XCTest
@testable import AHCore

/// What one finger does on the floating button, row by row of docs/specs/ios-developer-mode.md §5.2.
final class DeveloperPressTests: XCTestCase {
    private let layout = DeveloperLayout(size: CGSize(width: 393, height: 852), safeArea: DeveloperInsets(top: 59, bottom: 34))
    private let delay = DeveloperPressTuning.menuDelay

    /// The button's center (365, 438.5: halves are exact in binary, so "exactly 10 pt away" is exact) and the
    /// menu that opens next to it.
    private var center: CGPoint { DeveloperPlacement(edge: .right, y: 0.5).restingCenter(in: layout) }
    private var menu: DeveloperMenuLayout { .make(items: [.quickTodo], buttonCenter: center, in: layout) }
    private var hits: DeveloperHitMap {
        let size = DeveloperMetrics.buttonSize
        return DeveloperHitMap(button: CGRect(x: center.x - size / 2, y: center.y - size / 2, width: size, height: size), menu: menu)
    }
    private var onRow: CGPoint { CGPoint(x: menu.frame.midX, y: menu.frame.midY) }
    private let away = CGPoint(x: 60, y: 120)

    private func moved(_ point: CGPoint, by dx: CGFloat, _ dy: CGFloat = 0) -> CGPoint { CGPoint(x: point.x + dx, y: point.y + dy) }

    /// A press whose menu is open and whose finger is up.
    private func openMenu() -> DeveloperPress {
        var press = DeveloperPress()
        _ = press.down(at: center, hits: hits)
        _ = press.up(at: center, hits: hits)
        XCTAssertEqual(press.phase, .menuOpen)
        return press
    }

    // MARK: holding opens the menu and the finger chooses

    func testRestingOnTheButtonOpensTheMenuAndSlidingToARowAndLettingGoChoosesIt() {
        var press = DeveloperPress()
        XCTAssertEqual(press.down(at: center, hits: hits), [.scheduleMenuTimer(after: delay)])
        XCTAssertEqual(press.phase, .pending(quickReleaseOpens: true))

        XCTAssertEqual(press.menuTimerFired(), [.showMenu, .haptic(.opened)])
        XCTAssertEqual(press.phase, .menuHeld)

        XCTAssertEqual(press.move(to: onRow, hits: hits), [.highlight(.quickTodo), .haptic(.changed)])
        XCTAssertEqual(press.highlighted, .quickTodo)
        XCTAssertEqual(press.move(to: moved(onRow, by: 3), hits: hits), [], "still on the same row: nothing new")

        XCTAssertEqual(press.up(at: onRow, hits: hits), [.hideMenu, .haptic(.invoked), .invoke(.quickTodo)])
        XCTAssertEqual(press.phase, .idle)
        XCTAssertNil(press.highlighted)
    }

    func testSlidingOffTheMenuAndLettingGoChoosesNothing() {
        var press = DeveloperPress()
        _ = press.down(at: center, hits: hits)
        _ = press.menuTimerFired()
        XCTAssertEqual(press.move(to: onRow, hits: hits), [.highlight(.quickTodo), .haptic(.changed)])
        XCTAssertEqual(press.move(to: away, hits: hits), [.highlight(nil)], "leaving a row is not worth a tick")
        XCTAssertEqual(press.up(at: away, hits: hits), [.hideMenu])
        XCTAssertEqual(press.phase, .idle)
    }

    func testGoingBackToARowTicksAgain() {
        var press = DeveloperPress()
        _ = press.down(at: center, hits: hits)
        _ = press.menuTimerFired()
        _ = press.move(to: onRow, hits: hits)
        _ = press.move(to: away, hits: hits)
        XCTAssertEqual(press.move(to: onRow, hits: hits), [.highlight(.quickTodo), .haptic(.changed)])
    }

    func testLettingGoBackOnTheButtonLeavesTheMenuOpenForATap() {
        var press = DeveloperPress()
        _ = press.down(at: center, hits: hits)
        _ = press.menuTimerFired()
        XCTAssertEqual(press.up(at: center, hits: hits), [], "no row was chosen, the menu stays")
        XCTAssertEqual(press.phase, .menuOpen)

        var again = DeveloperPress()
        _ = again.down(at: center, hits: hits)
        _ = again.menuTimerFired()
        _ = again.move(to: onRow, hits: hits)
        XCTAssertEqual(again.move(to: center, hits: hits), [.highlight(nil)])
        XCTAssertEqual(again.up(at: center, hits: hits), [])
        XCTAssertEqual(again.phase, .menuOpen)
    }

    func testAHeldMenuCannotBeDraggedAway() {
        var press = DeveloperPress()
        _ = press.down(at: center, hits: hits)
        _ = press.menuTimerFired()
        XCTAssertEqual(press.move(to: away, hits: hits), [], "the finger moves, the button does not")
        XCTAssertEqual(press.phase, .menuHeld)
    }

    // MARK: a quick tap

    func testALetGoBeforeTheDelayOpensTheMenuAndLeavesItOpen() {
        var press = DeveloperPress()
        _ = press.down(at: center, hits: hits)
        XCTAssertEqual(press.up(at: center, hits: hits), [.cancelMenuTimer, .showMenu, .haptic(.opened)])
        XCTAssertEqual(press.phase, .menuOpen)
        XCTAssertEqual(press.menuTimerFired(), [], "the timer that was cancelled may still run out")
    }

    func testTappingARowOfAnOpenMenuChoosesIt() {
        var press = openMenu()
        XCTAssertEqual(press.down(at: onRow, hits: hits), [.highlight(.quickTodo), .haptic(.changed)])
        XCTAssertEqual(press.phase, .menuHeld)
        XCTAssertEqual(press.up(at: onRow, hits: hits), [.hideMenu, .haptic(.invoked), .invoke(.quickTodo)])
        XCTAssertEqual(press.phase, .idle)
    }

    func testTappingARowAndSlidingOffItBeforeLettingGoChoosesNothing() {
        var press = openMenu()
        _ = press.down(at: onRow, hits: hits)
        _ = press.move(to: away, hits: hits)
        XCTAssertEqual(press.up(at: away, hits: hits), [.hideMenu])
        XCTAssertEqual(press.phase, .idle)
    }

    func testTappingOutsideAnOpenMenuClosesItAndTheTouchGoesNoFurther() {
        var press = openMenu()
        XCTAssertEqual(press.down(at: away, hits: hits), [.hideMenu])
        XCTAssertEqual(press.phase, .swallowing)
        XCTAssertEqual(press.move(to: moved(away, by: 80), hits: hits), [], "not a drag, not a highlight")
        XCTAssertEqual(press.menuTimerFired(), [])
        XCTAssertEqual(press.up(at: moved(away, by: 80), hits: hits), [])
        XCTAssertEqual(press.phase, .idle)
    }

    func testTouchingTheButtonOfAnOpenMenuClosesItAndAQuickTapLeavesItClosed() {
        var press = openMenu()
        XCTAssertEqual(press.down(at: center, hits: hits), [.hideMenu, .scheduleMenuTimer(after: delay)])
        XCTAssertEqual(press.phase, .pending(quickReleaseOpens: false))
        XCTAssertEqual(press.up(at: center, hits: hits), [.cancelMenuTimer])
        XCTAssertEqual(press.phase, .idle)
    }

    func testTouchingTheButtonOfAnOpenMenuCanDragAtOnce() {
        var press = openMenu()
        _ = press.down(at: center, hits: hits)
        let target = moved(center, by: -40)
        XCTAssertEqual(press.move(to: target, hits: hits), [.cancelMenuTimer, .beginDrag, .drag(to: target)])
        XCTAssertEqual(press.phase, .dragging)
    }

    func testHoldingTheButtonOfAnOpenMenuOpensItAgain() {
        var press = openMenu()
        _ = press.down(at: center, hits: hits)
        XCTAssertEqual(press.menuTimerFired(), [.showMenu, .haptic(.opened)])
        XCTAssertEqual(press.phase, .menuHeld)
    }

    // MARK: dragging

    func testMovingAtOnceDragsTheButtonAndNeverOpensTheMenu() {
        var press = DeveloperPress()
        _ = press.down(at: center, hits: hits)
        let first = moved(center, by: -11)
        XCTAssertEqual(press.move(to: first, hits: hits), [.cancelMenuTimer, .beginDrag, .drag(to: first)])
        XCTAssertEqual(press.phase, .dragging)

        let next = CGPoint(x: 200, y: 300)
        XCTAssertEqual(press.move(to: next, hits: hits), [.drag(to: next)])
        XCTAssertEqual(press.menuTimerFired(), [], "a timer that was cancelled but runs out anyway does nothing")
        XCTAssertEqual(press.phase, .dragging)

        XCTAssertEqual(press.up(at: next, hits: hits), [.endDrag(at: next)])
        XCTAssertEqual(press.phase, .idle)
    }

    func testTheButtonKeepsTheOffsetItWasGrabbedAt() {
        var press = DeveloperPress()
        let grabbed = moved(center, by: 5, -3)
        _ = press.down(at: grabbed, hits: hits)
        let finger = CGPoint(x: 300, y: 400)
        XCTAssertEqual(press.move(to: finger, hits: hits), [.cancelMenuTimer, .beginDrag, .drag(to: CGPoint(x: 295, y: 403))])
        XCTAssertEqual(press.up(at: finger, hits: hits), [.endDrag(at: CGPoint(x: 295, y: 403))])
    }

    func testStrayingWithinTheSlopDoesNotDragAndTheMenuStillOpens() {
        var press = DeveloperPress()
        _ = press.down(at: center, hits: hits)
        XCTAssertEqual(press.move(to: moved(center, by: 6, 8), hits: hits), [], "10 pt away exactly: not farther than the slop")
        XCTAssertEqual(press.phase, .pending(quickReleaseOpens: true))
        XCTAssertEqual(press.menuTimerFired(), [.showMenu, .haptic(.opened)])
    }

    func testJustBeyondTheSlopIsADrag() {
        var press = DeveloperPress()
        _ = press.down(at: center, hits: hits)
        let target = moved(center, by: 0, 10.5)
        XCTAssertEqual(press.move(to: target, hits: hits), [.cancelMenuTimer, .beginDrag, .drag(to: target)])
    }

    // MARK: being interrupted

    func testTakingTheTouchAwayClosesWhatIsShowingAndRestoresADraggedButton() {
        var pending = DeveloperPress()
        _ = pending.down(at: center, hits: hits)
        XCTAssertEqual(pending.cancel(), [.cancelMenuTimer])
        XCTAssertEqual(pending.phase, .idle)

        var held = DeveloperPress()
        _ = held.down(at: center, hits: hits)
        _ = held.menuTimerFired()
        _ = held.move(to: onRow, hits: hits)
        XCTAssertEqual(held.cancel(), [.hideMenu])
        XCTAssertEqual(held.phase, .idle)
        XCTAssertNil(held.highlighted)

        var open = openMenu()
        XCTAssertEqual(open.cancel(), [.hideMenu], "the app losing the screen closes a menu nobody is touching too")
        XCTAssertEqual(open.phase, .idle)

        var dragging = DeveloperPress()
        _ = dragging.down(at: center, hits: hits)
        _ = dragging.move(to: moved(center, by: -50), hits: hits)
        XCTAssertEqual(dragging.cancel(), [.restorePosition])
        XCTAssertEqual(dragging.phase, .idle)

        var idle = DeveloperPress()
        XCTAssertEqual(idle.cancel(), [])
    }

    func testASecondFingerWhileDraggingPutsTheButtonBackAndStartsOver() {
        var press = DeveloperPress()
        _ = press.down(at: center, hits: hits)
        _ = press.move(to: moved(center, by: -50), hits: hits)
        XCTAssertEqual(press.down(at: center, hits: hits), [.restorePosition, .scheduleMenuTimer(after: delay)])
        XCTAssertEqual(press.phase, .pending(quickReleaseOpens: true))
    }

    func testATouchAwayFromTheButtonDoesNothingWhileNothingIsOpen() {
        var press = DeveloperPress()
        XCTAssertEqual(press.down(at: away, hits: hits), [])
        XCTAssertEqual(press.phase, .idle)
        XCTAssertEqual(press.up(at: away, hits: hits), [])
    }
}
