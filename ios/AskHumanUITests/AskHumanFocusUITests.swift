import XCTest

/// Answer fields must remain responsive when UIKit focus and SwiftUI updates overlap.
@MainActor
final class AskHumanFocusUITests: XCTestCase {
    private var app: XCUIApplication!
    private var lastEditor = 0

    override func setUp() async throws { continueAfterFailure = false }

    override func tearDown() async throws {
        guard let app else { return }
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func launch(_ request: String) {
        app = XCUIApplication()
        app.launchArguments = [
            "-AHMock", "YES", "-AHOpen", request, "-AHLatency", "0.05", "-AHAutoAdvance", "NO",
            "-AHWebMarkdown", "NO", "-AppleLanguages", "(en)", "-AppleLocale", "en_US",
        ]
        lastEditor = 0
        app.launch()
    }

    private func editor(_ index: Int) -> XCUIElement {
        let field = app.textViews["answer-text-\(index)"]
        let scroll = app.scrollViews.containing(.textView, identifier: "answer-text-\(index)").firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 15))
        for _ in 0..<8 {
            if field.isHittable {
                lastEditor = index
                return field
            }
            let surface = scroll.exists ? scroll : app!
            if index < lastEditor { surface.swipeDown(velocity: .fast) } else { surface.swipeUp(velocity: .fast) }
        }
        XCTAssertTrue(field.isHittable)
        return field
    }

    private func focus(_ index: Int) -> XCUIElement {
        let field = editor(index)
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        return field
    }

    func testTappingAnAnswerAfterSceneReactivationKeepsTypingResponsive() {
        launch("req-next")
        let first = focus(0)
        first.typeText("Before ")
        XCUIDevice.shared.press(.home)
        app.activate()
        let resumed = focus(0)
        resumed.typeText("After")
        XCTAssertTrue((resumed.value as? String)?.contains("Before") == true)
        XCTAssertTrue((resumed.value as? String)?.contains("After") == true)
    }

    func testSwitchingAnswerFieldsDoesNotTypeIntoAnEarlierQuestion() {
        launch("req-migration")
        focus(0).typeText("One ")
        focus(1).typeText("Two")
        let first = focus(0)
        first.typeText("Again")
        XCTAssertTrue((first.value as? String)?.contains("One") == true)
        XCTAssertTrue((first.value as? String)?.contains("Again") == true)
        XCTAssertFalse((first.value as? String)?.contains("Two") == true)
        XCTAssertEqual(editor(1).value as? String, "Two")
    }
}
