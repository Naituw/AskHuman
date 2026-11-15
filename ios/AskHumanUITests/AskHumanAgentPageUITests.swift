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

    /// A point in the margin left of the cards, at height `y`. The right margin is where the page's scroll indicator is: a
    /// finger that comes down within a few points of it takes hold of the indicator and drags the page to wherever the
    /// finger's height says, which is not a scroll under the finger.
    private func marginPoint(_ y: CGFloat) -> XCUICoordinate {
        point(max(4, element("watch-card").frame.minX - 8), y)
    }

    /// Moves the page until `id` is in the middle of the screen, with fingers that start in the margin beside the cards
    /// (a finger on a card is what the tests are about).
    private func bring(_ id: String, file: StaticString = #filePath, line: UInt = #line) {
        let screen = window.frame
        for _ in 0..<12 {
            let wanted = screen.height * 0.45 - element(id).frame.midY
            if abs(wanted) < screen.height * 0.1 { return }
            let travel = max(-screen.height * 0.3, min(screen.height * 0.3, wanted))
            let start = marginPoint(screen.height * (travel < 0 ? 0.62 : 0.38))
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
            let start = marginPoint(screen.height * 0.62)
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
        let update = element("watch-text-update")
        XCTAssertFalse(update.exists, "nothing is waiting yet")
        text.press(forDuration: 0.4)
        XCTAssertTrue(update.waitForExistence(timeout: 5), "the new text waits for the finger to leave")
        XCTAssertGreaterThanOrEqual(update.frame.height, fingertip, "\"New content\" is as tall as a fingertip")
        XCTAssertGreaterThanOrEqual(update.frame.width, fingertip, "\"New content\" is as wide as a fingertip")
    }

    /// The hold is a lease, not a lock: once the finger has left and the text has been held for a moment, what the Mac said
    /// meanwhile takes the place of the words by itself, without a tap on "New content".
    func testTheLatestTextIsReplacedByItselfAfterTheFingerLeaves() {
        launch(contentSize: nil, open: false)
        requireElement("agent-row-\(workingAgent)").tap()
        let text = requireElement("watch-text")
        let update = element("watch-text-update")
        text.press(forDuration: 0.4)
        XCTAssertTrue(update.waitForExistence(timeout: 5), "the new text waits while it is held")
        XCTAssertTrue(eventually(timeout: 25) { !update.exists }, "the new text replaced the old one without a tap")
    }

    /// "New content" is only there while the text is held: a finger on it, and the 3 seconds after. When the lease is over it
    /// goes, while the text it offered still waits for the 10 seconds between replacements. The view used to wake only when
    /// the text was due, so the offer stayed on the screen until the replacement, up to seven seconds after the lease.
    func testNewContentGoesWhenTheLeaseEndsWhileTheTextStillWaitsForTheInterval() throws {
        launch(contentSize: nil, open: false)
        requireElement("agent-row-\(workingAgent)").tap()
        let text = requireElement("watch-text")
        let shownAt = Date()
        let update = element("watch-text-update")
        let first = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "我已经定位到不稳定的原因")).firstMatch
        XCTAssertTrue(first.exists, "the first text is shown")
        text.press(forDuration: 0.4)
        XCTAssertTrue(update.waitForExistence(timeout: 5), "the new text waits while it is held")
        try XCTSkipIf(Date().timeIntervalSince(shownAt) > 5, "too slow a run to tell the end of the lease from the end of the interval")
        XCTAssertTrue(eventually(timeout: 8) { !update.exists }, "the offer went away when the lease ended")
        XCTAssertTrue(first.exists, "while the text it offered was still waiting for the interval")
    }

    /// Only a finger on the text holds it. One that scrolls the page somewhere else does not (it used to: any drag down the
    /// page held the text until "New content" was tapped).
    func testADragElsewhereOnThePageDoesNotHoldTheLatestText() {
        launch(contentSize: nil, open: false)
        requireElement("agent-row-\(workingAgent)").tap()
        requireElement("watch-text")
        let start = marginPoint(window.frame.height * 0.4)
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 120)), withVelocity: .slow, thenHoldForDuration: 0.1)
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -120)), withVelocity: .slow, thenHoldForDuration: 0.1)
        XCTAssertFalse(eventually(timeout: 2) { element("watch-text-update").exists }, "a drag beside the text held it")
    }

    // MARK: opening and closing

    /// Polls `condition` for a few seconds (the window's height animates for a quarter of a second, and the simulators are slow).
    private func eventually(timeout: TimeInterval = 6, _ condition: () -> Bool) -> Bool {
        let end = Date().addingTimeInterval(timeout)
        while Date() < end {
            if condition() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return condition()
    }

    /// Opening a long text makes its window taller, closing it gives the window back. Both samples (a thought, whose preview
    /// shows its end, and Markdown, whose preview shows its start) take the same way.
    private func opensAndCloses(_ sample: String) {
        launch(contentSize: nil, extra: ["-AHWatchText", sample])
        let toggle = requireElement("watch-text-expand")
        XCTAssertEqual(toggle.label, localized("Expand", "展开"))
        let excerpt = app.staticTexts[localized("Latest excerpt", "末尾片段")]
        XCTAssertEqual(excerpt.exists, sample == "thinking", "only the preview of a thought is marked as its end")
        // The window is between what is above it (a thought's name, or the live badge) and its button: measured between the
        // two, it does not depend on where the page is scrolled to (the page may have been moved to reach the button).
        let above = sample == "thinking" ? app.staticTexts[localized("Thinking", "思考")] : requireElement("watch-live")
        XCTAssertTrue(above.exists)
        func windowHeight() -> CGFloat { toggle.frame.minY - above.frame.maxY }
        let closed = windowHeight()
        shot("window-\(sample)-closed")
        toggle.tap()
        XCTAssertTrue(eventually { toggle.label == self.localized("Collapse", "收起") && windowHeight() > closed + 100 },
                      "the window grew to the whole text")
        XCTAssertFalse(excerpt.exists, "the whole text is not an excerpt")
        // Closed with the button at the top, which is the one that is in reach: the other one is at the end of the text, and on
        // a phone that is shorter than the text it is under the message bar or below the screen (closing from there is
        // `testClosingALongTextBringsItBackIntoSight`).
        let collapse = requireElement("watch-text-collapse", timeout: 5)
        shot("window-\(sample)-open")
        collapse.tap()
        XCTAssertTrue(eventually { toggle.label == self.localized("Expand", "展开") && abs(windowHeight() - closed) < 2 },
                      "the window went back to the preview")
        XCTAssertFalse(element("watch-text-collapse").exists, "a closed text has the one button")
        shot("window-\(sample)-closed-again")
    }

    func testALongThoughtOpensAndCloses() { opensAndCloses("thinking") }
    func testALongAnswerOpensAndCloses() { opensAndCloses("answer") }

    /// A text that fits in the preview has nothing to open, and is not an excerpt of anything.
    func testAShortTextHasNothingToOpen() {
        launch(contentSize: nil, extra: ["-AHWatchText", "short"])
        requireElement("watch-text")
        XCTAssertFalse(element("watch-text-expand").waitForExistence(timeout: 3), "there is nothing to open")
        XCTAssertFalse(app.staticTexts[localized("Latest excerpt", "末尾片段")].exists, "the thought is whole")
    }

    /// What a fingertip needs of a button (Apple's guideline): the one that opens the text was 24 × 14 points, and it was
    /// missed more often than not.
    private let fingertip: CGFloat = 43.5

    /// The whole 44 points are the button, not only the words in the middle of them: a tap in the corner of that area opens
    /// the text, which is also what a tap beside the words does.
    func testTheButtonThatOpensTheTextIsAsBigAsAFingertipAllOverItsArea() {
        launch(contentSize: nil, extra: ["-AHWatchText", "long"])
        let toggle = requireElement("watch-text-expand")
        XCTAssertGreaterThanOrEqual(toggle.frame.height, fingertip, "tall enough")
        XCTAssertGreaterThanOrEqual(toggle.frame.width, fingertip, "wide enough")
        // The words are in the middle of the area's height: its top and bottom edges are where only the area is.
        toggle.coordinate(withNormalizedOffset: CGVector(dx: 0.08, dy: 0.92)).tap()
        XCTAssertTrue(eventually { toggle.label == self.localized("Collapse", "收起") }, "a tap at the corner of the area opened the text")
        let collapse = requireElement("watch-text-collapse", timeout: 5)
        XCTAssertGreaterThanOrEqual(collapse.frame.height, fingertip, "the button at the top is as tall as a fingertip")
        XCTAssertGreaterThanOrEqual(collapse.frame.width, fingertip, "the button at the top is as wide as a fingertip")
        collapse.coordinate(withNormalizedOffset: CGVector(dx: 0.08, dy: 0.08)).tap()
        XCTAssertTrue(eventually { toggle.label == self.localized("Expand", "展开") }, "a tap at the corner of its area closed it")
    }

    /// An opened long text is closed from its top, which is where the page is after opening it: its other button is at the end
    /// of the text, a screen and a half further down.
    func testAnOpenedLongTextIsClosedFromItsTopWithoutScrollingToItsEnd() {
        launch(contentSize: nil, extra: ["-AHWatchText", "long"])
        let toggle = requireElement("watch-text-expand")
        XCTAssertFalse(element("watch-text-collapse").exists, "closed, the text has the one button")
        toggle.tap()
        let collapse = requireElement("watch-text-collapse", timeout: 5)
        XCTAssertTrue(eventually { toggle.label == self.localized("Collapse", "收起") }, "the text opened")
        XCTAssertTrue(collapse.isHittable, "the button at the top of the opened text is on the screen")
        XCTAssertFalse(toggle.isHittable, "the one at its end is not: the text is taller than the screen")
        shot("window-open-long-top")
        collapse.tap()
        XCTAssertTrue(eventually { toggle.label == self.localized("Expand", "展开") && !element("watch-text-collapse").exists },
                      "the text closed from its top")
        XCTAssertTrue(toggle.isHittable, "back in the overview, with its button")
    }

    /// The window of the text: between what is above it and its bottom button.
    private func windowHeight(of toggle: XCUIElement, above: XCUIElement) -> CGFloat { toggle.frame.minY - above.frame.maxY }

    /// The height of the window once it has stopped changing: opening or replacing a text moves it over a quarter of a second,
    /// and the button's label has changed long before that (the first look at it is somewhere on the way).
    private func settledWindowHeight(of toggle: XCUIElement, above: XCUIElement) -> CGFloat {
        var last = windowHeight(of: toggle, above: above)
        for _ in 0..<20 {
            Thread.sleep(forTimeInterval: 0.3)
            let now = windowHeight(of: toggle, above: above)
            if abs(now - last) < 1 { return now }
            last = now
        }
        return last
    }

    /// Opening a text does not hold it (2026-11-15: it did, until it was closed, and then only a tap on "New content" showed
    /// what the Agent said next). The next paragraph replaces an opened text by itself, and the text stays open.
    func testAnOpenedTextIsReplacedByItselfAndStaysOpen() {
        // One frame a second, so that the answer (the third frame) comes three seconds after the page opened. At the usual
        // pace it came after a third of a second, and on a busy machine the first render of the long text took longer than
        // that: the page then showed the answer first, and there was nothing left to replace it.
        launch(contentSize: nil, extra: ["-AHWatchText", "long", "-AHWatchTextNext", "answer", "-AHWatchTick", "1"])
        let toggle = requireElement("watch-text-expand")
        let above = requireElement("watch-live")
        toggle.tap()
        XCTAssertTrue(eventually { toggle.label == self.localized("Collapse", "收起") }, "the text opened")
        let whole = settledWindowHeight(of: toggle, above: above)
        XCTAssertGreaterThan(whole, 400, "the first text is the long one, opened")
        // Ten seconds after the first text was shown the answer takes its place: shorter, so the window gets shorter.
        XCTAssertTrue(eventually(timeout: 30) { self.windowHeight(of: toggle, above: above) < whole - 150 },
                      "the next text replaced the opened one without a tap")
        XCTAssertEqual(toggle.label, localized("Collapse", "收起"), "it is still open")
        XCTAssertTrue(element("watch-text-collapse").exists, "and can be closed from its top")
        XCTAssertFalse(element("watch-text-update").exists, "nothing is waiting any more")
        shot("window-replaced-while-open")
    }

    /// A long text read down to its end is closed from there: the page goes back up to it instead of being left at the end of
    /// the shorter page, where something else is.
    func testClosingALongTextBringsItBackIntoSight() {
        launch(contentSize: nil, extra: ["-AHWatchText", "long"])
        let toggle = requireElement("watch-text-expand")
        let field = requireElement("interject-field")
        toggle.tap()
        XCTAssertTrue(eventually { toggle.label == self.localized("Collapse", "收起") }, "the text opened")
        // Down the text to its end (its top out of sight, its button on the screen above the message bar, which floats over
        // the page: a tap under it is a tap in its text field), with fingers in the margin beside the card.
        let screen = window.frame
        for _ in 0..<14 where !(element("watch-text").frame.minY < 0 && toggle.frame.maxY < field.frame.minY) {
            let start = marginPoint(screen.height * 0.62)
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -screen.height * 0.3)), withVelocity: .slow, thenHoldForDuration: 0.2)
        }
        XCTAssertLessThan(element("watch-text").frame.minY, 0, "the top of the text is above the screen")
        XCTAssertLessThan(toggle.frame.maxY, field.frame.minY, "its button is on the screen, above the message bar")
        shot("window-read-to-the-end")
        toggle.tap()
        XCTAssertTrue(
            eventually { element("watch-text").frame.minY >= 0 && toggle.label == self.localized("Expand", "展开") && toggle.frame.maxY <= screen.height },
            "the text is back in sight, with its button"
        )
        shot("window-closed-from-the-end")
    }
}
