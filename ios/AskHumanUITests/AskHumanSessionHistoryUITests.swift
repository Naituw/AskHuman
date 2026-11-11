import XCTest

/// The earlier exchanges of a request's session (`docs/specs/ios-session-history.md`), through the real UI over the mock
/// Macs' `sessionHistory` scenario, in English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`).
///
/// The scenario's requests: `req-history-long` (a session of 23 exchanges), `req-history-confirm` (a permission request of the
/// same session), `req-history-short` (3), `req-history-empty` (none), `req-history-none` (names no session) and
/// `req-history-old-mac` (on a Mac that cannot read history back).
@MainActor
final class AskHumanSessionHistoryUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        if let run = testRun, run.totalFailureCount > 0, app != nil { shot("failed-" + name.filter { $0.isLetter }) }
    }

    // MARK: helpers

    @discardableResult
    private func launch(
        _ request: String = "req-history-long", scenario: String = "sessionHistory", extra: [String] = [], contentSize: String? = nil,
        dark: Bool = false, autoAdvance: Bool = false
    ) -> XCUIApplication {
        app?.terminate()
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        var arguments = [
            "-AHMock", "YES", "-AHLatency", "0.05", "-AHScenario", scenario, "-AHWebMarkdown", "NO", "-AHOpen", request,
            "-AHAutoAdvance", autoAdvance ? "YES" : "NO", "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ] + extra
        if let contentSize { arguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategory" + contentSize] }
        if dark { arguments += ["-AppleInterfaceStyle", "Dark"] }
        app.launchArguments = arguments
        app.launch()
        self.app = app
        _ = requireElement("request-menu")
        return app
    }

    private func localized(_ en: String, _ zh: String) -> String { language == "en" ? en : zh }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    @discardableResult
    private func requireElement(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let found = element(id)
        let exists = found.waitForExistence(timeout: timeout)
        if !exists { add(XCTAttachment(string: app.debugDescription)) }
        XCTAssertTrue(exists, "\(id) did not appear", file: file, line: line)
        return found
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"], !folder.isEmpty {
            let url = URL(fileURLWithPath: folder).appendingPathComponent("history-\(language)-\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
            return
        }
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// The scroll view of the page: the biggest of the app's (the first can be the horizontal scroller of a code block in
    /// the request, or the suggestion bar of the keyboard).
    private func pageScroll() -> XCUIElement {
        app.scrollViews.allElementsBoundByIndex.max { $0.frame.height < $1.frame.height } ?? app.scrollViews.firstMatch
    }

    private func cards() -> XCUIElementQuery {
        app.descendants(matching: .any).matching(identifier: "session-exchange")
    }

    private func waitFor(_ message: String, timeout: TimeInterval = 15, file: StaticString = #filePath, line: UInt = #line, _ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return }
            Thread.sleep(forTimeInterval: 0.1)
        }
        XCTFail(message, file: file, line: line)
    }

    private func waitForCards(atLeast count: Int, timeout: TimeInterval = 15, file: StaticString = #filePath, line: UInt = #line) {
        waitFor("fewer than \(count) cards appeared (\(cards().count))", timeout: timeout, file: file, line: line) { cards().count >= count }
    }

    /// A slow pull from near the top, held a moment before it lets go.
    private func pull(from: CGFloat = 0.25, to: CGFloat = 0.8, inWindow: Bool = false) {
        // With the keyboard up the first scroll view of the app can be the one of its suggestion bar.
        let space: XCUIElement = inWindow ? app : pageScroll()
        let start = space.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: from))
        let end = space.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: to))
        start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.15)
    }

    /// Drags the page down until what is at its top stops changing.
    private func dragToTop(maxSwipes: Int = 40) {
        let scroll = pageScroll()
        var last = -1.0
        for _ in 0..<maxSwipes {
            scroll.swipeDown(velocity: .fast)
            let top = cards().count > 0 ? cards().firstMatch.frame.minY : element("session-thread-start").frame.minY
            if abs(top - last) < 0.5 { return }
            last = top
        }
    }

    private func openMenuItem() {
        requireElement("request-menu").tap()
        let item = element("request-menu-session-history")
        XCTAssertTrue(item.waitForExistence(timeout: 5), "the menu has no session history item")
        item.tap()
    }

    private func settle() { Thread.sleep(forTimeInterval: 1.2) }

    private func backToInboxAndReopen(_ id: String) {
        app.navigationBars.buttons.element(boundBy: 0).tap()
        requireElement("inbox-row-\(id)").tap()
        _ = requireElement("request-menu")
    }

    // MARK: the pull

    /// The page of most requests: shorter than a screen.
    func testPullingDownLoadsTheLatestPageAboveAndTheRequestStaysPut() {
        launch()
        let title = requireElement("request-title")
        let before = title.frame.minY
        shot("1-before")
        XCTAssertEqual(cards().count, 0, "nothing of the history is loaded before it is asked for")
        pull()
        waitForCards(atLeast: 8)
        settle()
        shot("2-after-the-pull")
        XCTAssertEqual(cards().count, 8, "one page of eight")
        XCTAssertEqual(title.frame.minY, before, accuracy: 1, "the request moved on screen")
        XCTAssertFalse(element("session-thread-start").exists, "there is more to find")
        // The page rests within its range, so a touch moves nothing: dragging the request up has nowhere to go.
        pageScroll().swipeUp(velocity: .slow)
        Thread.sleep(forTimeInterval: 0.8)
        XCTAssertEqual(title.frame.minY, before, accuracy: 1, "the page was out of its range and came back at the touch")
    }

    /// A page taller than a screen: nothing is different, the request stays exactly where it was.
    func testPullingDownOnATallPageKeepsTheRequestPutToo() {
        launch("req-history-tall")
        let title = requireElement("request-title")
        let before = title.frame.minY
        pull()
        waitForCards(atLeast: 8)
        settle()
        shot("tall-page-after-the-pull")
        XCTAssertEqual(cards().count, 8)
        XCTAssertEqual(title.frame.minY, before, accuracy: 1, "the request moved on screen")
    }

    func testTheThreadIsReadByDraggingDownAndNewestIsNextToTheRequest() {
        launch()
        pull()
        waitForCards(atLeast: 8)
        settle()
        let newest = cards().element(boundBy: cards().count - 1)
        let title = requireElement("request-title")
        XCTAssertLessThan(newest.frame.maxY, title.frame.minY, "the newest exchange is above the request")
        pageScroll().swipeDown(velocity: .slow)
        settle()
        shot("3-reading-the-newest")
        XCTAssertTrue(newest.frame.minY > 0, "the newest exchange is on screen after a short drag")
        XCTAssertTrue(
            element("session-thread-current").exists, "a line says where the request starts"
        )
    }

    func testPullingAgainAddsOlderPagesUntilTheStartIsFound() {
        launch()
        pull()
        waitForCards(atLeast: 8)
        dragToTop()
        pull()
        waitForCards(atLeast: 16)
        dragToTop()
        pull()
        waitForCards(atLeast: 23)
        settle()
        XCTAssertEqual(cards().count, 23, "the whole session, and not one exchange twice")
        let start = requireElement("session-thread-start")
        XCTAssertEqual(start.label, localized("No earlier records", "没有更早的记录了"))
        shot("4-start-of-the-session")
        // Nothing is asked any more: another pull adds nothing and does not wait.
        dragToTop()
        pull()
        Thread.sleep(forTimeInterval: 1)
        XCTAssertEqual(cards().count, 23)
    }

    func testAShortSessionOnAShortPageCanBePulledToo() {
        launch("req-history-short")
        let title = requireElement("request-title")
        let before = title.frame.minY
        pull()
        waitForCards(atLeast: 3)
        settle()
        XCTAssertEqual(cards().count, 3)
        XCTAssertTrue(element("session-thread-start").exists, "three exchanges are all there is")
        XCTAssertEqual(title.frame.minY, before, accuracy: 1, "the request stayed put")
        XCTAssertLessThan(cards().element(boundBy: 2).frame.maxY, title.frame.minY, "the newest exchange is next to the request")
    }

    func testAConfirmationPageHasTheThreadToo() {
        launch("req-history-confirm")
        let title = requireElement("request-title")
        let before = title.frame.minY
        pull()
        waitForCards(atLeast: 8)
        settle()
        shot("confirm-after-the-pull")
        XCTAssertEqual(cards().count, 8)
        XCTAssertEqual(title.frame.minY, before, accuracy: 1, "the confirmation stayed put")
        XCTAssertLessThan(cards().element(boundBy: 7).frame.maxY, title.frame.minY, "the thread is above the confirmation")
        XCTAssertTrue(title.isHittable, "the confirmation is still on screen")
    }

    // MARK: the menu

    func testTheMenuLoadsTheHistoryAndScrollsToItsNewestExchange() {
        launch()
        let before = requireElement("request-title").frame.minY
        openMenuItem()
        waitForCards(atLeast: 8)
        settle()
        shot("menu-after")
        let newest = cards().element(boundBy: cards().count - 1)
        XCTAssertTrue(newest.frame.minY > 40 && newest.frame.minY < 420, "the newest exchange is at the top of the screen: \(newest.frame)")
        XCTAssertGreaterThan(requireElement("request-title").frame.minY, before, "the request is below it now")
    }

    func testTheMenuOnALoadedThreadOnlyScrolls() {
        launch()
        pull()
        waitForCards(atLeast: 8)
        dragToTop()
        let top = cards().firstMatch.frame.minY
        openMenuItem()
        settle()
        XCTAssertEqual(cards().count, 8, "nothing more was loaded")
        let newest = cards().element(boundBy: cards().count - 1)
        XCTAssertTrue(newest.frame.minY > 40 && newest.frame.minY < 420, "it scrolled to the newest exchange: \(newest.frame)")
        XCTAssertNotEqual(cards().firstMatch.frame.minY, top)
    }

    func testTheMenuOnAShortSessionShowsWhereItStarts() {
        launch("req-history-short")
        openMenuItem()
        waitForCards(atLeast: 3)
        settle()
        XCTAssertEqual(cards().count, 3)
        XCTAssertTrue(element("session-thread-start").exists)
    }

    func testASessionWithNothingEarlierSaysSoAtTheTop() {
        launch("req-history-empty")
        openMenuItem()
        let start = requireElement("session-thread-start")
        settle()
        XCTAssertEqual(cards().count, 0)
        XCTAssertEqual(start.label, localized("No earlier records", "没有更早的记录了"))
        XCTAssertTrue(start.isHittable || start.frame.minY > 0, "the row is on screen: \(start.frame)")
        shot("empty-session")
    }

    // MARK: no entry

    func testARequestWithoutASessionHasNoEntry() {
        for request in ["req-history-none", "req-history-old-mac"] {
            launch(request)
            requireElement("request-menu").tap()
            XCTAssertFalse(element("request-menu-session-history").waitForExistence(timeout: 2), "\(request) has a menu item")
            // A tap outside closes the menu.
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.6)).tap()
            Thread.sleep(forTimeInterval: 0.5)
            pull()
            Thread.sleep(forTimeInterval: 1)
            XCTAssertEqual(cards().count, 0, "\(request) loaded a thread")
        }
    }

    func testTheDemoScenarioIsTheSamePageAsBefore() {
        launch("req-migration", scenario: "demo")
        requireElement("request-menu").tap()
        XCTAssertFalse(element("request-menu-session-history").waitForExistence(timeout: 2))
        XCTAssertTrue(element("request-menu-add-todo").exists, "the rest of the menu is as it was")
    }

    // MARK: the cards

    func testACardOpensInPlaceAndClosesAgain() {
        launch()
        openMenuItem()
        waitForCards(atLeast: 8)
        settle()
        let card = cards().element(boundBy: cards().count - 1)
        XCTAssertEqual(card.value as? String, localized("Collapsed", "已折叠"))
        let folded = card.frame.height
        card.tap()
        Thread.sleep(forTimeInterval: 0.6)
        XCTAssertEqual(card.value as? String, localized("Expanded", "已展开"))
        XCTAssertGreaterThanOrEqual(card.frame.height, folded, "an open card is not shorter than a folded one")
        shot("card-open")
        card.tap()
        Thread.sleep(forTimeInterval: 0.6)
        XCTAssertEqual(card.value as? String, localized("Collapsed", "已折叠"))
        XCTAssertEqual(card.frame.height, folded, accuracy: 1)
    }

    func testACardSpeaksItsExchangeAsOne() {
        launch()
        openMenuItem()
        waitForCards(atLeast: 8)
        settle()
        // The newest exchange of the long session (index 0): its message and its one question, answered.
        let newest = cards().element(boundBy: cards().count - 1)
        XCTAssertTrue(newest.label.contains("cache layer"), newest.label)
        XCTAssertTrue(newest.label.contains("Keep the TTL"), newest.label)
        XCTAssertTrue(newest.label.contains("Yes, keep it"), newest.label)
        XCTAssertTrue(newest.label.contains(localized("Mac popup", "Mac 弹窗")), "where it was answered: \(newest.label)")
    }

    // MARK: failing

    func testAMacThatDoesNotAnswerShowsARowAndTryingAgainLoads() {
        launch(extra: ["-AHHistoryFail", "1"])
        pull()
        let row = requireElement("session-thread-error")
        settle()
        XCTAssertTrue(row.exists)
        XCTAssertTrue(app.staticTexts[localized("Can't reach the Mac", "连不上 Mac")].exists, "the row says why")
        XCTAssertEqual(cards().count, 0)
        shot("error-row")
        requireElement("session-thread-retry").tap()
        waitForCards(atLeast: 8)
        settle()
        XCTAssertFalse(element("session-thread-error").exists, "the row is gone once it worked")
        XCTAssertEqual(cards().count, 8)
    }

    func testTheMenuOnAMacThatDoesNotAnswerShowsTheRowAtTheTop() {
        launch(extra: ["-AHHistoryFail", "1"])
        openMenuItem()
        let retry = requireElement("session-thread-retry")
        settle()
        XCTAssertTrue(retry.isHittable, "the row is on screen")
        XCTAssertEqual(cards().count, 0)
    }

    // MARK: the way back

    func testBackToTheCurrentRequestAppearsFarAboveAndGoesBack() {
        launch()
        openMenuItem()
        waitForCards(atLeast: 8)
        settle()
        let title = requireElement("request-title")
        XCTAssertFalse(element("session-return").exists, "the request is within a screen")
        dragToTop()
        let capsule = requireElement("session-return", timeout: 5)
        shot("the-way-back")
        XCTAssertTrue(capsule.isHittable)
        capsule.tap()
        Thread.sleep(forTimeInterval: 1.2)
        XCTAssertTrue(title.isHittable, "the request is in view again")
        XCTAssertFalse(element("session-return").exists, "and nothing is offered any more")
    }

    // MARK: with the page

    func testLeavingWhileReadingTheThreadComesBackToTheTopOfTheRequestWithoutIt() {
        launch()
        let top = requireElement("request-title").frame.minY
        pull()
        waitForCards(atLeast: 8)
        for _ in 0..<3 { pageScroll().swipeDown(velocity: .fast) }
        Thread.sleep(forTimeInterval: 0.6)
        backToInboxAndReopen("req-history-long")
        Thread.sleep(forTimeInterval: 1)
        XCTAssertEqual(cards().count, 0, "the thread went with the page")
        XCTAssertEqual(requireElement("request-title").frame.minY, top, accuracy: 2, "the page is at the top of the request")
    }

    func testTheMenuIsNotPulledBackByAReadingPositionWaitingToBeRestored() {
        launch()
        let scroll = pageScroll()
        scroll.swipeUp(velocity: .fast)
        Thread.sleep(forTimeInterval: 0.6)
        backToInboxAndReopen("req-history-long")
        openMenuItem()
        waitForCards(atLeast: 8)
        Thread.sleep(forTimeInterval: 2)
        let newest = cards().element(boundBy: cards().count - 1)
        XCTAssertTrue(newest.frame.minY > 40 && newest.frame.minY < 420, "the page did not scroll to the newest exchange: \(newest.frame)")
    }

    func testNextQuestionAndAnsweringStillWorkWithTheThreadAbove() throws {
        launch("req-history-long")
        pull()
        waitForCards(atLeast: 8)
        settle()
        let option = requireElement("option-0-0")
        // A single question: there is no "Next"; the bar's count is what answering changes.
        let count = element("answered-count")
        let before = count.exists ? count.label : ""
        for _ in 0..<6 where !option.isHittable { pageScroll().swipeUp(velocity: .slow) }
        XCTAssertTrue(option.isHittable, "the first option can be reached with the thread above")
        option.tap()
        Thread.sleep(forTimeInterval: 0.6)
        if count.exists { XCTAssertNotEqual(count.label, before) }
        XCTAssertTrue(element("send-button").isEnabled, "the answer can be sent")
    }

    func testPullingWithTheKeyboardUpDoesNotJump() throws {
        launch()
        let field = app.textFields.firstMatch.exists ? app.textFields.firstMatch : app.textViews.firstMatch
        guard field.waitForExistence(timeout: 3) else { throw XCTSkip("the request has no text field") }
        for _ in 0..<4 where !field.isHittable { pageScroll().swipeUp(velocity: .slow) }
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        var loaded = false
        for _ in 0..<10 {
            pull(from: 0.15, to: 0.5, inWindow: true)
            if cards().count >= 8 { loaded = true; break }
            Thread.sleep(forTimeInterval: 0.5)
            if cards().count >= 8 { loaded = true; break }
        }
        shot("keyboard-after")
        XCTAssertTrue(loaded, "the pull with the keyboard up never loaded the thread")
    }

    // MARK: ended requests

    func testLoadingTheHistoryOnAnEndedPageStopsItsCountdown() {
        launch(extra: ["-AHEndedCountdown", "6", "-AHEndAfter", "2", "-AHEndHow", "answeredElsewhere"], autoAdvance: true)
        requireElement("finished-notice", timeout: 40)
        openMenuItem()
        waitForCards(atLeast: 8)
        // Well past the countdown (6 seconds from when the strip was shown).
        Thread.sleep(forTimeInterval: 9)
        XCTAssertTrue(element("finished-notice").exists, "the page was not moved on")
        XCTAssertGreaterThanOrEqual(cards().count, 8)
        shot("ended-page-with-history")
        // What was loaded can still be opened on an ended page.
        let card = cards().element(boundBy: cards().count - 1)
        card.tap()
        Thread.sleep(forTimeInterval: 0.6)
        XCTAssertEqual(card.value as? String, localized("Expanded", "已展开"))
    }

    func testWithoutLoadingTheHistoryAnEndedPageStillMovesOn() {
        launch(extra: ["-AHEndedCountdown", "2", "-AHEndAfter", "2", "-AHEndHow", "answeredElsewhere"], autoAdvance: true)
        requireElement("finished-notice", timeout: 40)
        waitFor("the ended page did not move on by itself", timeout: 15) { !element("finished-notice").exists }
    }

    // MARK: looks

    func testTheLargestTextStillHasCardsThatReadAsCards() {
        launch(contentSize: "AccessibilityXXXL")
        openMenuItem()
        waitForCards(atLeast: 8)
        settle()
        shot("largest-text")
        let card = cards().element(boundBy: cards().count - 1)
        XCTAssertGreaterThan(card.frame.width, app.frame.width * 0.7, "a card is squeezed: \(card.frame)")
    }

    func testOnAnIPadThePullStillLoadsAndTheRequestStaysPut() throws {
        try XCTSkipIf(UIDevice.current.userInterfaceIdiom != .pad, "A phone has its own layout.")
        launch()
        let title = requireElement("request-title")
        let before = title.frame.minY
        pull()
        waitForCards(atLeast: 8)
        settle()
        shot("ipad-after-the-pull")
        XCTAssertEqual(title.frame.minY, before, accuracy: 1)
    }

    func testDarkMode() {
        launch(dark: true)
        openMenuItem()
        waitForCards(atLeast: 8)
        settle()
        shot("dark")
        XCTAssertEqual(cards().count, 8)
    }
}
