import XCTest
@testable import AHCore

/// Where the floating button rests and snaps (docs/specs/ios-developer-mode.md §5.3).
final class DeveloperPlacementTests: XCTestCase {
    /// A phone with a Dynamic Island and a home indicator. The button's center ranges from
    /// 59 + 8 + 22 = 89 to 852 − 34 − 8 − 22 = 788; on the right it rests at 393 − 6 − 22 = 365.
    private let layout = DeveloperLayout(size: CGSize(width: 393, height: 852), safeArea: DeveloperInsets(top: 59, bottom: 34))

    private func assertPoint(
        _ point: CGPoint, _ x: CGFloat, _ y: CGFloat, _ message: String = "", file: StaticString = #filePath, line: UInt = #line
    ) {
        XCTAssertEqual(point.x, x, accuracy: 0.001, "x. \(message)", file: file, line: line)
        XCTAssertEqual(point.y, y, accuracy: 0.001, "y. \(message)", file: file, line: line)
    }

    // MARK: remembering

    func testTheStandardPlacementIsOnTheRightAboveTheMiddle() {
        XCTAssertEqual(DeveloperPlacement.standard, DeveloperPlacement(edge: .right, y: 0.4))
    }

    func testAPlacementSurvivesStorageAndDataOutOfRangeIsPulledIn() throws {
        let placement = DeveloperPlacement(edge: .left, y: 0.25)
        let data = try JSONEncoder().encode(placement)
        XCTAssertEqual(try JSONDecoder().decode(DeveloperPlacement.self, from: data), placement)

        let wild = try JSONDecoder().decode(DeveloperPlacement.self, from: Data(#"{"edge":"left","y":7}"#.utf8))
        XCTAssertEqual(wild, DeveloperPlacement(edge: .left, y: 1), "a height beyond the screen is the bottom of it")
        XCTAssertEqual(DeveloperPlacement(edge: .right, y: -3).y, 0)
        XCTAssertEqual(DeveloperPlacement(edge: .right, y: .nan).y, DeveloperPlacement.standard.y)
    }

    func testStoredDataThatIsNotAPlacementIsNotOne() {
        XCTAssertThrowsError(try JSONDecoder().decode(DeveloperPlacement.self, from: Data(#"{"edge":"up","y":0.5}"#.utf8)))
        XCTAssertThrowsError(try JSONDecoder().decode(DeveloperPlacement.self, from: Data("garbage".utf8)))
    }

    // MARK: resting

    func testItRestsBesideTheEdgeAtTheHeightItWasLeftAt() {
        assertPoint(DeveloperPlacement.standard.restingCenter(in: layout), 365, 89 + 0.4 * 699, "right, 40%")
        assertPoint(DeveloperPlacement(edge: .left, y: 0).restingCenter(in: layout), 28, 89, "left, top of the free height")
        assertPoint(DeveloperPlacement(edge: .right, y: 1).restingCenter(in: layout), 365, 788, "right, bottom of the free height")
    }

    func testTheSafeAreaAtTheSidesIsRespected() {
        let sideways = DeveloperLayout(
            size: CGSize(width: 852, height: 393), safeArea: DeveloperInsets(top: 0, left: 59, bottom: 21, right: 59)
        )
        XCTAssertEqual(DeveloperPlacement.edgeX(.left, in: sideways), 59 + 6 + 22)
        XCTAssertEqual(DeveloperPlacement.edgeX(.right, in: sideways), 852 - 59 - 6 - 22)
    }

    func testAScreenTooShortForTheButtonStillGivesAPoint() {
        let tiny = DeveloperLayout(size: CGSize(width: 320, height: 50), safeArea: DeveloperInsets(top: 20, bottom: 10))
        let range = DeveloperPlacement.verticalRange(in: tiny)
        XCTAssertEqual(range.lowerBound, range.upperBound)
        XCTAssertEqual(DeveloperPlacement.standard.restingCenter(in: tiny).y, range.lowerBound)
    }

    // MARK: keyboard

    func testTheButtonRisesAboveTheKeyboardOnlyWhenItWouldBeCoveredAndTheRememberedPlacementStays() {
        let placement = DeveloperPlacement.standard  // rests at y = 368.6
        var withKeyboard = layout

        withKeyboard.keyboardTop = 520
        assertPoint(placement.displayedCenter(in: withKeyboard), 365, 368.6, "the keyboard is far below: it stays")

        withKeyboard.keyboardTop = 300
        assertPoint(placement.displayedCenter(in: withKeyboard), 365, 300 - 8 - 22, "it sits 8 above the keyboard")

        withKeyboard.keyboardTop = nil
        assertPoint(placement.displayedCenter(in: withKeyboard), 365, 368.6, "the keyboard is gone: it is back")
        XCTAssertEqual(placement, .standard, "nothing was remembered")
    }

    func testTheKeyboardNeverPushesTheButtonOutOfTheSafeArea() {
        var huge = layout
        huge.keyboardTop = 100  // would put the button at y = 70, above the safe area's top margin (89)
        assertPoint(DeveloperPlacement.standard.displayedCenter(in: huge), 365, 89)
    }

    // MARK: dragging

    func testLettingGoSnapsToTheNearestSideAtTheHeightItWasLetGoAt() {
        let left = DeveloperPlacement.snapped(from: CGPoint(x: 100, y: 89 + 0.25 * 699), in: layout)
        XCTAssertEqual(left.edge, .left)
        XCTAssertEqual(left.y, 0.25, accuracy: 0.0001)

        let right = DeveloperPlacement.snapped(from: CGPoint(x: 300, y: 788), in: layout)
        XCTAssertEqual(right, DeveloperPlacement(edge: .right, y: 1))

        XCTAssertEqual(DeveloperPlacement.snapped(from: CGPoint(x: 196.4, y: 300), in: layout).edge, .left)
        XCTAssertEqual(DeveloperPlacement.snapped(from: CGPoint(x: 196.5, y: 300), in: layout).edge, .right, "the middle goes right")
    }

    func testLettingGoAboveOrBelowTheFreeHeightLandsAtItsEnds() {
        XCTAssertEqual(DeveloperPlacement.snapped(from: CGPoint(x: 10, y: -50), in: layout).y, 0)
        XCTAssertEqual(DeveloperPlacement.snapped(from: CGPoint(x: 10, y: 5000), in: layout).y, 1)
    }

    func testTheButtonIsKeptInsideTheScreenWhileItIsDragged() {
        assertPoint(DeveloperPlacement.clampedDragCenter(CGPoint(x: -40, y: -40), in: layout), 28, 89)
        assertPoint(DeveloperPlacement.clampedDragCenter(CGPoint(x: 900, y: 900), in: layout), 365, 788)
        assertPoint(DeveloperPlacement.clampedDragCenter(CGPoint(x: 200, y: 400), in: layout), 200, 400, "inside, it follows the finger")
    }
}
