import XCTest

/// The page of one Agent session (`docs/specs/ios-channel.md` §6.7): the live card, its latest text, the plan and the steps,
/// through the real UI over the mock Macs, in English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`). The demo's working
/// Agent `claude:cc-flaky` has a live view with a text, a plan of three and three steps.
@MainActor
final class AskHumanAgentPageUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }
    private let workingAgent = "claude:cc-flaky"

    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        if let run = testRun, run.totalFailureCount > 0, app != nil { shot("failed-" + name.filter { $0.isLetter }) }
    }

    // MARK: helpers

    /// `contentSize` makes the page longer than the screen, as a long text does on a real phone.
    @discardableResult
    private func launch(contentSize: String? = "AccessibilityL", open: Bool = true, extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        var arguments = [
            "-AHMock", "YES", "-AHScenario", "demo", "-AHLatency", "0.05", "-AHAutoAdvance", "NO", "-AHWebMarkdown", "NO",
            "-AHTab", "agents",
            "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ]
        if open { arguments += ["-AHOpenAgent", workingAgent] }
        if let contentSize { arguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategory" + contentSize] }
        app.launchArguments = arguments + extra
        app.launch()
        self.app = app
        return app
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    @discardableResult
    private func requireElement(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let found = element(id)
        XCTAssertTrue(found.waitForExistence(timeout: timeout), "\(id) did not appear", file: file, line: line)
        return found
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"], !folder.isEmpty {
            let url = URL(fileURLWithPath: folder).appendingPathComponent("\(language)-\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
            return
        }
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = "\(language)-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func localized(_ en: String, _ zh: String) -> String { language == "en" ? en : zh }

    private var window: XCUIElement { app.windows.firstMatch }

    /// A point of the window, in points from its top left corner.
    private func point(_ x: CGFloat, _ y: CGFloat) -> XCUICoordinate {
        window.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: x, dy: y))
    }

    /// Moves the page until `id` is in the middle of the screen, with fingers that start in the margin beside the cards
    /// (a finger on a card is what the tests are about).
    private func bring(_ id: String, file: StaticString = #filePath, line: UInt = #line) {
        let screen = window.frame
        for _ in 0..<12 {
            let wanted = screen.height * 0.45 - element(id).frame.midY
            if abs(wanted) < screen.height * 0.1 { return }
            let travel = max(-screen.height * 0.3, min(screen.height * 0.3, wanted))
            let start = point(element("watch-card").frame.maxX + 6, screen.height * (travel < 0 ? 0.62 : 0.38))
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: travel)), withVelocity: .slow, thenHoldForDuration: 0.2)
        }
        XCTFail("\(id) could not be brought to the middle of the screen", file: file, line: line)
    }

    /// Whether the page moves under a finger that comes down at `id`'s middle, stays for `linger` seconds and then drags
    /// 150 points, up and (when the page was at its end) down.
    private func scrolls(from id: String, linger: TimeInterval = 0.05) -> Bool {
        let marker = element("watch-card")
        let before = marker.frame.minY
        for direction: CGFloat in [-1, 1] {
            let start = element(id).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            start.press(forDuration: linger, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 150 * direction)), withVelocity: .slow, thenHoldForDuration: 0.1)
            if abs(marker.frame.minY - before) > 20 { return true }
        }
        return false
    }

    // MARK: scrolling

    /// On iOS 27 a drag gesture on the latest text took the page's scroll away from a finger that started on it.
    func testAFingerThatStartsOnTheLatestTextStillScrollsThePage() {
        launch()
        requireElement("watch-card")
        requireElement("watch-text")
        bring("watch-text")
        XCTAssertTrue(scrolls(from: "watch-text"), "the page moved under a finger that started on the latest text")
    }

    func testAFingerThatLingersOnTheLatestTextBeforeScrollingStillScrollsThePage() {
        launch()
        requireElement("watch-card")
        requireElement("watch-text")
        bring("watch-text")
        XCTAssertTrue(scrolls(from: "watch-text", linger: 0.4), "the page moved under a finger that stayed on the latest text")
    }

    /// Fingers all the way down the middle of the screen, whatever is under them.
    private func sweep(_ place: String, file: StaticString = #filePath, line: UInt = #line) {
        let screen = window.frame
        for fraction in stride(from: 0.2, through: 0.8, by: 0.15) {
            let marker = element("watch-card")
            let before = marker.frame.minY
            var moved = false
            for direction: CGFloat in [-1, 1] where !moved {
                let start = point(screen.width * 0.5, screen.height * fraction)
                start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 100 * direction)), withVelocity: .slow, thenHoldForDuration: 0.1)
                moved = abs(marker.frame.minY - before) > 15
            }
            XCTAssertTrue(moved, "\(place): the page did not move under a finger at \(Int(fraction * 100))% of the screen's height", file: file, line: line)
        }
    }

    /// The header, the latest text and its buttons, then (at the end of the page) the plan and the steps.
    func testThePageScrollsUnderAFingerAnywhereOnTheLiveCard() {
        continueAfterFailure = true
        launch()
        requireElement("watch-card")
        requireElement("watch-text")
        bring("watch-text")
        sweep("around the latest text")
        // The end of the page: fingers that start in the margin until it stops moving.
        let screen = window.frame
        var last = element("watch-card").frame.minY
        for _ in 0..<12 {
            let start = point(element("watch-card").frame.maxX + 6, screen.height * 0.62)
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -screen.height * 0.3)), withVelocity: .slow, thenHoldForDuration: 0.2)
            let now = element("watch-card").frame.minY
            if abs(now - last) < 2 { break }
            last = now
        }
        sweep("at the end of the page")
    }

    // MARK: reading

    /// A finger that stays on the text is reading it: what the Mac says next waits behind "New content" instead of
    /// taking the place of the words under the finger.
    func testTheLatestTextHoldsStillWhileAFingerIsOnIt() {
        // The page is opened by hand, so that the 10 seconds the text is left alone for start when the finger comes.
        launch(contentSize: nil, open: false)
        requireElement("agent-row-\(workingAgent)").tap()
        let text = requireElement("watch-text")
        // (The buttons are found by their names: the identifier of the text's container is the one they report.)
        let update = app.buttons[localized("New content", "有新内容")]
        XCTAssertFalse(update.exists, "nothing is waiting yet")
        text.press(forDuration: 0.4)
        XCTAssertTrue(update.waitForExistence(timeout: 5), "the new text waits for the finger to leave")
    }
}
