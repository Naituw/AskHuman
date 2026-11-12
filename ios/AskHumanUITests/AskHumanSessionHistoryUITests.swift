import XCTest
import QuartzCore

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
        if dark {
            // The launch argument `-AppleInterfaceStyle Dark` leaves the app light; the device's appearance is what it follows.
            XCUIDevice.shared.appearance = .dark
            addTeardownBlock { XCUIDevice.shared.appearance = .light }
        }
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

    /// A page offers its history once the Mac's abilities are known, a moment after the page is up (a good deal longer on a
    /// busy machine). A touch before that is a touch on a short page, which does not move at all. A page that offers none (no
    /// session, a Mac too old) never shows the element, and the wait just ends.
    private func waitForTheHistoryToBeOffered() {
        _ = element("session-pull-state").waitForExistence(timeout: 4)
    }

    /// A slow pull from near the top, held a moment before it lets go.
    private func pull(from: CGFloat = 0.25, to: CGFloat = 0.8, inWindow: Bool = false) {
        waitForTheHistoryToBeOffered()
        // With the keyboard up the first scroll view of the app can be the one of its suggestion bar.
        let space: XCUIElement = inWindow ? app : pageScroll()
        let start = space.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: from))
        let end = space.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: to))
        start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.15)
    }

    /// Drags the page down until the top of the thread is in view, each drag no longer than what is left to go. (A drag that
    /// goes on past the top is a pull, and loads another page: a swipe of the length the test runner gives it would do that
    /// every time it reaches the top.)
    private func dragToTop(maxDrags: Int = 40) {
        let scroll = pageScroll()
        let height = scroll.frame.height
        for _ in 0..<maxDrags {
            // The top of the thread is the row that says where it starts, once there is one, or else its oldest card; the page
            // is as far down as it can be when that is a little below the navigation bar.
            let row = element("session-thread-start")
            guard row.exists || cards().count > 0 else { return }
            let top = row.exists ? row.frame.minY : cards().firstMatch.frame.minY
            let needed = (row.exists ? 130 : 108) - top
            if needed <= 12 { return }
            let travel = min(needed, height * 0.6)
            let start = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
            let end = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2 + travel / height))
            start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .default, thenHoldForDuration: 0.2)
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
        if UIDevice.current.userInterfaceIdiom == .pad {
            // The list stays beside the page: leaving is opening another request, coming back is opening this one again.
            let others = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'inbox-row-' AND identifier != %@", "inbox-row-\(id)"))
            XCTAssertTrue(others.firstMatch.waitForExistence(timeout: 10), "no other request in the list")
            others.firstMatch.tap()
            _ = requireElement("request-menu")
            Thread.sleep(forTimeInterval: 1)
        } else {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
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
        // The request went one row down when the pull was let go (the row of the loading line), and did not move when the
        // history came.
        XCTAssertEqual(title.frame.minY, before + heldRoom, accuracy: 1, "the request moved on screen")
        XCTAssertFalse(element("session-thread-start").exists, "there is more to find")
        // The page rests within its range: a touch can take the request back up to where it was before the pull, no further
        // (a page that rests out of its range jumps at the first touch).
        pageScroll().swipeUp(velocity: .slow)
        Thread.sleep(forTimeInterval: 0.8)
        XCTAssertGreaterThanOrEqual(title.frame.minY, before - 1, "the page was out of its range and came back at the touch")
        XCTAssertLessThanOrEqual(title.frame.minY, before + heldRoom + 1)
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
        XCTAssertEqual(title.frame.minY, before + heldRoom, accuracy: 1, "the request moved on screen")
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
        XCTAssertEqual(title.frame.minY, before + heldRoom, accuracy: 1, "the request moved on screen")
        XCTAssertLessThan(cards().element(boundBy: 2).frame.maxY, title.frame.minY, "the newest exchange is next to the request")
        XCTAssertLessThanOrEqual(element("session-thread-start").frame.maxY, cards().element(boundBy: 0).frame.minY + 1, "the row that says where it starts is above the oldest exchange")
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
        XCTAssertEqual(title.frame.minY, before + heldRoom, accuracy: 1, "the confirmation moved on screen")
        XCTAssertLessThan(cards().element(boundBy: 7).frame.maxY, title.frame.minY, "the thread is above the confirmation")
        XCTAssertTrue(title.isHittable, "the confirmation is still on screen")
    }

    // MARK: the menu

    func testTheMenuLoadsTheHistoryAndScrollsToItsNewestExchange() {
        launch(extra: ["-AHPullProbe", probePath("menu")])
        let before = requireElement("request-title").frame.minY
        openMenuItem()
        waitForCards(atLeast: 8)
        settle()
        shot("menu-after")
        let newest = cards().element(boundBy: cards().count - 1)
        // (The page's own record of its motion goes with a failure: where it was, and when, on the way.)
        let motion = readProbe("menu").suffix(14).map { "\($0.time) off=\($0.values["off"] ?? "-") size=\($0.values["size"] ?? "-") req=\($0.values["req"] ?? "-") dec=\($0.values["dec"] ?? "-")" }.joined(separator: "\n")
        XCTAssertTrue(newest.frame.minY > 40 && newest.frame.minY < 420, "the newest exchange is at the top of the screen: \(newest.frame)\n\(motion)")
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
            // Far enough to be taken (about 145 pt of finger travel past the top), still above the keyboard.
            pull(from: 0.14, to: 0.56, inWindow: true)
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

    /// The same by pulling: whichever way the history is asked for, the page is not moved away from it.
    func testPullingTheHistoryOnAnEndedPageStopsItsCountdownToo() {
        launch(extra: ["-AHEndedCountdown", "8", "-AHEndAfter", "2", "-AHEndHow", "answeredElsewhere"], autoAdvance: true)
        requireElement("finished-notice", timeout: 40)
        pull()
        waitForCards(atLeast: 8)
        // Well past the countdown (8 seconds from when the strip was shown).
        Thread.sleep(forTimeInterval: 10)
        XCTAssertTrue(element("finished-notice").exists, "the page was not moved on")
        XCTAssertGreaterThanOrEqual(cards().count, 8)
        shot("ended-page-pulled")
    }

    func testWithoutLoadingTheHistoryAnEndedPageStillMovesOn() {
        // A countdown long enough to be seen by a query that takes its time on a busy machine.
        launch(extra: ["-AHEndedCountdown", "6", "-AHEndAfter", "2", "-AHEndHow", "answeredElsewhere"], autoAdvance: true)
        requireElement("finished-notice", timeout: 40)
        waitFor("the ended page did not move on by itself", timeout: 30) { !element("finished-notice").exists }
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
        XCTAssertEqual(title.frame.minY, before + heldRoom, accuracy: 1)
    }

    /// With the two columns side by side the pull's words are centred where the page's content is, which is where the loading
    /// line that takes their place is. From iOS 26 the sidebar is a panel over the detail's scroll view, which spans the whole
    /// window and has the sidebar's width as its leading safe area: a header laid across the scroll view is centred on the
    /// window, partly under the sidebar, and the words jump sideways when the row replaces them.
    func testWithTwoColumnsThePullHeaderIsCenteredWhereTheLoadingLineIs() throws {
        launch(extra: ["-AHHistoryDelay", "3", "-AHPullProbe", probePath("columns")])
        let window = app.windows.firstMatch.frame
        try XCTSkipIf(window.width < 700, "The columns sit side by side only in a wide window.")
        _ = requireElement("request-title")
        pull(travel: 300, hold: 1.0)
        waitForThePullToTakeHold(probe: "columns")
        let row = requireElement("session-thread-loading", timeout: 3)
        let rowCenter = row.frame.midX
        shot("columns-loading")
        // The strips of the window the header lay across (left edge, width), one for each frame in which it was on show.
        let strips = readProbe("columns").compactMap { line -> (left: Double, width: Double)? in
            guard let strip = line.values["hx"], strip != "-" else { return nil }
            let numbers = strip.split(separator: ",").compactMap { Double($0) }
            return numbers.count == 2 ? (numbers[0], numbers[1]) : nil
        }
        XCTAssertFalse(strips.isEmpty, "the probe never saw the header")
        for strip in strips {
            XCTAssertEqual(
                strip.left + strip.width / 2, rowCenter, accuracy: 2,
                "the header lay across \(strip.left)…\(strip.left + strip.width) of a window \(window.width) wide; the loading line is centred on \(rowCenter)")
        }
    }

    func testDarkMode() {
        launch(dark: true)
        openMenuItem()
        waitForCards(atLeast: 8)
        settle()
        shot("dark")
        XCTAssertEqual(cards().count, 8)
    }

    // MARK: the pull as the page draws it (R1: docs/plans/ios-session-history.md)

    /// The room a pull holds open for its loading line, in points, at the default text size.
    private let heldRoom: CGFloat = 64

    /// A pull of this many points of finger travel, held a moment before it lets go. The page gives way less than the finger
    /// moves: a travel of 70 points opens it by about 35, one of 240 by well over the threshold.
    private func pull(travel: CGFloat, hold: TimeInterval = 0.15) {
        waitForTheHistoryToBeOffered()
        let scroll = pageScroll()
        let height = scroll.frame.height
        let start = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
        let end = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2 + travel / height))
        start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: hold)
    }

    private func pullState() -> String { element("session-pull-state").label }

    /// Waits for the page to take a pull. When it does not, the failure says what the page saw of the touch (the last lines
    /// of the probe) and a screenshot is kept: a pull the page never starts to follow is a different thing from one it
    /// follows and does not load.
    private func waitForThePullToTakeHold(probe name: String, timeout: TimeInterval = 5, file: StaticString = #filePath, line: UInt = #line) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if pullState() == "loading" { return }
            Thread.sleep(forTimeInterval: 0.1)
        }
        shot("pull-not-taken")
        let seen = readProbe(name).suffix(6).map { "\($0.time) \($0.note ?? "") \($0.values.sorted { $0.key < $1.key }.map { "\($0.key)=\($0.value)" }.joined(separator: " "))" }
        XCTFail("the pull never took hold (state \(pullState())); the page saw:\n" + seen.joined(separator: "\n"), file: file, line: line)
    }

    /// What the app wrote of the page's motion (`-AHPullProbe`): a line for every frame in which something changed.
    private struct ProbeLine {
        var time: Double
        var note: String?
        var values: [String: String] = [:]
        var request: Double? { values["req"].flatMap(Double.init) }
        var offset: Double? { values["off"].flatMap(Double.init) }
    }

    private func probePath(_ name: String) -> String { "/tmp/ah-pull-probe-\(name).log" }

    private func readProbe(_ name: String) -> [ProbeLine] {
        guard let text = try? String(contentsOfFile: probePath(name), encoding: .utf8) else { return [] }
        return text.split(separator: "\n").compactMap { raw in
            let line = raw.trimmingCharacters(in: .whitespaces)
            guard let space = line.firstIndex(of: " "), let time = Double(line[..<space]) else { return nil }
            let rest = line[line.index(after: space)...].trimmingCharacters(in: .whitespaces)
            if rest.hasPrefix("#") { return ProbeLine(time: time, note: String(rest.dropFirst(2))) }
            var values: [String: String] = [:]
            for token in rest.split(separator: " ") {
                let pair = token.split(separator: "=", maxSplits: 1)
                if pair.count == 2 { values[String(pair[0])] = String(pair[1]) }
            }
            return ProbeLine(time: time, values: values)
        }
    }

    /// Where the request was on screen in every frame from the note `prefix` until `seconds` later, and the last place it was
    /// before the note.
    private func requestAround(_ prefix: String, for seconds: Double = 4.0, in log: [ProbeLine]) -> (before: Double?, during: [Double])? {
        guard let mark = log.first(where: { $0.note?.hasPrefix(prefix) == true }) else { return nil }
        let before = log.last(where: { $0.time < mark.time && $0.request != nil })?.request
        let during = log.filter { $0.time >= mark.time && $0.time <= mark.time + seconds }.compactMap(\.request)
        return (before, during)
    }

    /// The thread came in above the request in the same layout pass as the request was put back: in no frame of the next
    /// seconds was the request anywhere but where it had been. (A main thread that is starved lays the thread out a second
    /// late, and the first frame the probe writes after it comes then: the window has to reach that far.)
    private func assertTheRequestDidNotMoveWhenTheThreadCameIn(
        _ log: [ProbeLine], within seconds: Double = 4.0, file: StaticString = #filePath, line: UInt = #line
    ) {
        guard let around = requestAround("thread comes in", for: seconds, in: log), let before = around.before else {
            return XCTFail("the probe has no record of the thread coming in (\(log.count) lines)", file: file, line: line)
        }
        XCTAssertFalse(around.during.isEmpty, file: file, line: line)
        for place in around.during {
            XCTAssertEqual(place, before, accuracy: 1, "a frame had the request at \(place), it was at \(before)", file: file, line: line)
        }
    }

    /// From the lift on, the request moves only the way the page springs back (up the screen, never down and never in a jump)
    /// and ends where the room of the loading row puts it.
    private func assertTheLoadingRowTookItsPlaceWithoutAJump(_ log: [ProbeLine], file: StaticString = #filePath, line: UInt = #line) {
        guard let mark = log.first(where: { $0.note == "loading starts" }), let initial = log.first(where: { $0.request != nil })?.request else {
            return XCTFail("the probe has no record of the loading starting", file: file, line: line)
        }
        // Where the request is once the room of the row is under it: one room below where the page began (the probe measures
        // the top of the request's area, not its title).
        let rest = initial + Double(heldRoom)
        let frames = log.filter { $0.time >= mark.time && $0.time <= mark.time + 1.2 }.compactMap(\.request)
        XCTAssertGreaterThan(frames.count, 3, "too few frames after the lift: \(frames)", file: file, line: line)
        var last: Double?
        var biggestStep = 0.0
        for place in frames {
            if let last {
                XCTAssertLessThanOrEqual(place, last + 0.5, "the request moved down the screen (\(last) -> \(place))", file: file, line: line)
                biggestStep = max(biggestStep, last - place)
            }
            last = place
        }
        XCTAssertLessThan(biggestStep, 45, "the request jumped by \(biggestStep) points in a frame: \(frames)", file: file, line: line)
        XCTAssertEqual(frames.last ?? -1, rest, accuracy: 1.5, "the page did not come to rest on the room of the loading row", file: file, line: line)
    }

    /// A pull let go past its threshold holds the room of a row open at the top of the page; the loading takes as long as it
    /// takes and the history then comes in above the request, which does not move.
    func testAPullLetGoPastTheThresholdHoldsItsRoomAndTheHistoryComesInWithoutMovingTheRequest() {
        launch(extra: ["-AHHistoryDelay", "6", "-AHPullProbe", probePath("armed")])
        let title = requireElement("request-title")
        let start = title.frame.minY
        XCTAssertEqual(pullState(), "idle")
        shot("pull-1-before")
        pull(travel: 240)
        waitFor("the pull never took hold (state \(pullState()))", timeout: 5) { pullState() == "loading" }
        // The line is read once (the lit copy of its words that sweeps over it is not a second line).
        let loading = requireElement("session-thread-loading", timeout: 3)
        XCTAssertEqual(loading.label, localized("Loading history…", "正在加载历史…"))
        // The room opens and the page settles on it.
        Thread.sleep(forTimeInterval: 0.9)
        // The line shimmers: a highlight sweeps over its words, so two looks at it a moment apart are not the same picture.
        // (With Reduce Motion it stands still on purpose; `testWithReduceMotionTheIconStaysStill` covers that run.)
        let firstLook = loading.screenshot().pngRepresentation
        Thread.sleep(forTimeInterval: 0.45)
        if ProcessInfo.processInfo.environment["AH_UI_REDUCE_MOTION"] != "1" {
            XCTAssertNotEqual(firstLook, loading.screenshot().pngRepresentation, "the loading line stands still")
        }
        shot("pull-2-holding")
        let held = title.frame.minY
        XCTAssertEqual(held - start, heldRoom, accuracy: 3, "the request is one room lower than it was")
        XCTAssertEqual(cards().count, 0, "nothing has come yet")
        waitForCards(atLeast: 8, timeout: 10)
        Thread.sleep(forTimeInterval: 1.2)
        shot("pull-3-arrived")
        XCTAssertEqual(cards().count, 8)
        XCTAssertEqual(title.frame.minY, held, accuracy: 1, "the request moved when the history came in")
        XCTAssertEqual(pullState(), "idle")

        let log = readProbe("armed")
        XCTAssertTrue(log.contains { $0.values["pull"] == "armed" }, "the pull was never armed")
        assertTheRequestDidNotMoveWhenTheThreadCameIn(log)
        assertTheLoadingRowTookItsPlaceWithoutAJump(log)
    }

    /// A main thread that is starved lays the thread out late (here the app is held up for 1.4 s between the page getting
    /// ready for the thread and the thread going in): the request does not move for that, because the page keeps putting
    /// it right until the layout has come, however late. With a compensation window counted from the request to put the
    /// thread in (0.5 s) this test fails: the request ends up a thread's height lower.
    func testAThreadThatIsLaidOutLateStillDoesNotMoveTheRequest() {
        // The answer comes late (5 s) so that the page has come to rest by then, whatever the speed of the simulator: a page
        // that is still springing back when the main thread is held up goes on springing, and that is not what is tested.
        launch(extra: ["-AHHistoryDelay", "5", "-AHPullStall", "1.4", "-AHPullProbe", probePath("stall")])
        _ = requireElement("request-title")
        pull(travel: 240)
        waitForThePullToTakeHold(probe: "stall")
        waitForCards(atLeast: 8, timeout: 15)
        Thread.sleep(forTimeInterval: 1.2)
        let log = readProbe("stall")
        guard let came = log.first(where: { $0.note == "thread comes in" })?.time,
              let next = log.first(where: { $0.time > came && $0.request != nil })?.time
        else { return XCTFail("the probe has no frame after the thread came in") }
        XCTAssertGreaterThan(next - came, 1.2, "the main thread was not held up, so this run proves nothing")
        assertTheRequestDidNotMoveWhenTheThreadCameIn(log)
        XCTAssertEqual(cards().count, 8)
    }

    /// Let go short of the threshold, a pull loads nothing and leaves nothing behind.
    func testAPullLetGoShortOfTheThresholdDoesNothing() {
        launch(extra: ["-AHHistoryDelay", "0.3", "-AHPullProbe", probePath("short")])
        let title = requireElement("request-title")
        let start = title.frame.minY
        pull(travel: 70)
        Thread.sleep(forTimeInterval: 1.5)
        XCTAssertEqual(pullState(), "idle")
        XCTAssertEqual(cards().count, 0, "a short pull loaded the history")
        XCTAssertEqual(title.frame.minY, start, accuracy: 1, "the page did not come back")
        let log = readProbe("short")
        XCTAssertTrue(log.contains { $0.values["pull"] == "pulling" }, "the header was never shown")
        XCTAssertFalse(log.contains { $0.values["pull"] == "armed" || $0.values["pull"] == "loading" }, "the pull was armed")
    }

    /// The history comes while the person has scrolled the page down somewhere else: what is on screen does not move.
    func testTheHistoryComingWhileTheRequestIsScrolledDoesNotMoveWhatIsOnScreen() {
        launch("req-history-tall", extra: ["-AHHistoryDelay", "4", "-AHPullProbe", probePath("scrolled")])
        _ = requireElement("request-title")
        pull(travel: 240)
        waitFor("the pull never took hold (state \(pullState()))", timeout: 5) { pullState() == "loading" }
        Thread.sleep(forTimeInterval: 0.9)
        // The row scrolls away with the page: it is page content, and the page is not locked.
        pageScroll().swipeUp(velocity: .slow)
        Thread.sleep(forTimeInterval: 0.8)
        shot("scrolled-1-while-loading")
        waitForCards(atLeast: 8, timeout: 10)
        Thread.sleep(forTimeInterval: 1.2)
        shot("scrolled-2-arrived")
        let log = readProbe("scrolled")
        assertTheRequestDidNotMoveWhenTheThreadCameIn(log)
        if let before = requestAround("thread comes in", in: log)?.before {
            XCTAssertLessThan(before, 60, "the page was scrolled well down while it loaded (the request is at \(before))")
        }
    }

    /// "Try again" puts the same row back to the loading line, and nothing on the page moves, then or when the history comes.
    func testTryingAgainPutsTheSameRowBackToTheLoadingLine() {
        launch(extra: ["-AHHistoryFail", "1", "-AHHistoryDelay", "3", "-AHPullProbe", probePath("retry")])
        let title = requireElement("request-title")
        pull(travel: 240)
        waitFor("the pull never took hold (state \(pullState()))", timeout: 5) { pullState() == "loading" }
        let retry = requireElement("session-thread-retry", timeout: 12)
        Thread.sleep(forTimeInterval: 0.8)
        let held = title.frame.minY
        let rowMiddle = requireElement("session-thread-error").frame.midY
        retry.tap()
        let loading = requireElement("session-thread-loading", timeout: 3)
        XCTAssertFalse(element("session-thread-error").exists, "the row still says it failed")
        XCTAssertEqual(pullState(), "loading")
        XCTAssertEqual(title.frame.minY, held, accuracy: 1, "the request moved when the line came back")
        XCTAssertEqual(loading.frame.midY, rowMiddle, accuracy: 2, "the loading line is not in the row that said it failed")
        shot("retry-loading")
        waitForCards(atLeast: 8, timeout: 12)
        Thread.sleep(forTimeInterval: 1.2)
        XCTAssertEqual(cards().count, 8)
        XCTAssertEqual(title.frame.minY, held, accuracy: 1, "the request moved when the history came in")
        XCTAssertFalse(element("session-thread-error").exists)
        XCTAssertEqual(pullState(), "idle")
        assertTheRequestDidNotMoveWhenTheThreadCameIn(readProbe("retry"))
    }

    /// The words of the header follow the pull: "Pull down…" on the way, "Release…" past the threshold, and the wording for
    /// earlier records once there is a thread above the request.
    func testTheHeaderSaysPullDownOnTheWayAndReleaseOnceItIsFarEnough() {
        launch(extra: ["-AHHistoryDelay", "0.2", "-AHPullProbe", probePath("words")])
        _ = requireElement("request-title")
        pull(travel: 240)
        waitForCards(atLeast: 8)
        Thread.sleep(forTimeInterval: 1.2)
        dragToTop()
        Thread.sleep(forTimeInterval: 1)
        if cards().count < 16 {
            pull(travel: 240)
            waitForCards(atLeast: 16)
        }
        Thread.sleep(forTimeInterval: 1.2)
        let looks = readProbe("words").compactMap { $0.values["hdr"] }
        for look in ["p0", "r0", "p1", "r1"] { XCTAssertTrue(looks.contains(look), "the header never showed \(look): \(Set(looks))") }
        // On the way the words come first, and the ones for "release" only after them.
        if let first = looks.firstIndex(of: "p0"), let release = looks.firstIndex(of: "r0") { XCTAssertLessThan(first, release) }
        // Strong enough to read only while a finger pulls (the header is not on the page otherwise).
        let log = readProbe("words")
        let shown = log.filter { $0.values["hdr"].map { $0 != "-" } == true }
        for line in shown {
            XCTAssertTrue(line.values["drag"] == "1" || line.values["pull"] == "loading", "the header showed without a finger pulling: \(line.values)")
        }
        // The icon turns as the page is pulled (not when Reduce Motion is on: see the test for it).
        if log.first(where: { $0.values["rm"] != nil })?.values["rm"] == "0" {
            XCTAssertTrue(shown.contains { abs(Double($0.values["turn"] ?? "0") ?? 0) > 0.05 }, "the icon never turned")
        }
    }

    /// With Reduce Motion the icon does not turn. The simulator's setting has to be on for this run (`AH_UI_REDUCE_MOTION=1`
    /// says it is); the probe says whether the app saw it.
    func testWithReduceMotionTheIconStaysStill() throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["AH_UI_REDUCE_MOTION"] == "1", "needs the simulator's Reduce Motion switched on")
        launch(extra: ["-AHHistoryDelay", "5", "-AHPullProbe", probePath("motion")])
        _ = requireElement("request-title")
        pull(travel: 240)
        waitFor("the pull never took hold (state \(pullState()))", timeout: 5) { pullState() == "loading" }
        // The loading line has no highlight sweeping over it: two looks at it a moment apart are the same picture.
        let loading = requireElement("session-thread-loading", timeout: 3)
        Thread.sleep(forTimeInterval: 0.9)
        let firstLook = loading.screenshot().pngRepresentation
        Thread.sleep(forTimeInterval: 0.45)
        XCTAssertEqual(firstLook, loading.screenshot().pngRepresentation, "the loading line moves with Reduce Motion on")
        waitForCards(atLeast: 8, timeout: 12)
        Thread.sleep(forTimeInterval: 1.2)
        let log = readProbe("motion")
        XCTAssertTrue(log.contains { $0.values["rm"] == "1" }, "the app did not see Reduce Motion")
        let shown = log.filter { $0.values["hdr"].map { $0 != "-" } == true }
        XCTAssertFalse(shown.isEmpty, "the header was never shown")
        for line in shown { XCTAssertEqual(Double(line.values["turn"] ?? "1") ?? 1, 0, accuracy: 0.001, "the icon turned: \(line.values)") }
        XCTAssertEqual(cards().count, 8)
    }

    /// A finger on the page when the answer comes holds the history back: it comes in once the page is let go, and the request
    /// does not move then either.
    func testAFingerStillOnThePageHoldsTheHistoryBackUntilItLetsGo() {
        let gate = "/tmp/ah-history-gate-finger"
        try? FileManager.default.removeItem(atPath: gate)
        addTeardownBlock { try? FileManager.default.removeItem(atPath: gate) }
        launch(extra: ["-AHHistoryDelay", "0", "-AHHistoryGate", gate, "-AHPullProbe", probePath("finger")])
        _ = requireElement("request-title")
        pull(travel: 240)
        waitForThePullToTakeHold(probe: "finger")
        // The answer is held at the (mock) Mac until the page has seen a finger on it, so the finger is certainly there
        // when the answer is ready, however late the test runner puts it on.
        let answerReady = openTheGateOnceAFingerIsOnThePage(probe: "finger", gate: gate)
        // A finger that drags the page a little and then stays on it, well past the moment the answer is ready. (A finger
        // that only rests on the page is not tracked by it: a long press on its text is not a scroll.)
        let scroll = pageScroll()
        let from = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.7))
        let to = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.55))
        from.press(forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 5)
        waitForCards(atLeast: 8, timeout: 10)
        Thread.sleep(forTimeInterval: 1.2)
        let log = readProbe("finger")
        guard let readyAt = answerReady.time else { return XCTFail("the page never saw the finger, so the answer was never let through") }
        guard let came = log.first(where: { $0.note == "thread comes in" })?.time else { return XCTFail("the thread never came in") }
        // When the page saw the finger go: the first frame without it after the last frame with it. (The test's own clock
        // would be late: the press call returns only once the page the finger let go of has stopped moving.)
        guard let lastTouched = log.last(where: { $0.values["trk"] == "1" && $0.time < came })?.time,
              let leftAt = log.first(where: { $0.values["trk"] == "0" && $0.time > lastTouched })?.time
        else { return XCTFail("the page never saw the finger go") }
        XCTAssertGreaterThan(leftAt, readyAt + 1, "the finger was gone before the answer was ready: nothing was held back")
        XCTAssertGreaterThanOrEqual(came, leftAt, "the thread came in with a finger on the page")
        assertTheRequestDidNotMoveWhenTheThreadCameIn(log)
    }

    /// When the gate was opened (a clock the app's probe shares), once it has been.
    private final class GateOpened: @unchecked Sendable {
        private let lock = NSLock()
        private var stamp: Double?
        var time: Double? { lock.lock(); defer { lock.unlock() }; return stamp }
        func set(_ value: Double) { lock.lock(); stamp = value; lock.unlock() }
    }

    /// Lets the (mock) Mac's answer through a second after the page has seen a finger on it while the load was in flight.
    /// The call that puts the finger on blocks this thread, so a background queue watches the probe the page writes.
    private func openTheGateOnceAFingerIsOnThePage(probe name: String, gate: String) -> GateOpened {
        let opened = GateOpened()
        let path = probePath(name)
        DispatchQueue.global().async {
            let begin = CACurrentMediaTime()
            while CACurrentMediaTime() - begin < 90 {
                if Self.aFingerIsOnThePageWhileLoading(probe: path) {
                    Thread.sleep(forTimeInterval: 1)
                    opened.set(CACurrentMediaTime())
                    FileManager.default.createFile(atPath: gate, contents: Data())
                    return
                }
                Thread.sleep(forTimeInterval: 0.1)
            }
        }
        return opened
    }

    /// A frame of the probe after the load began in which the page tracks a finger while it holds the room of the loading row.
    nonisolated private static func aFingerIsOnThePageWhileLoading(probe path: String) -> Bool {
        guard let text = try? String(contentsOfFile: path, encoding: .utf8), let began = text.range(of: "# loading starts") else { return false }
        return text[began.upperBound...].split(separator: "\n").contains { $0.contains(" trk=1 ") && $0.contains(" hold=1 ") }
    }

    /// A touch of the page during the loading (the spring back from the pull, the hold) breaks nothing.
    func testATouchOfThePageRightAfterTheLiftBreaksNothing() {
        launch(extra: ["-AHHistoryDelay", "1.5", "-AHPullProbe", probePath("touch")])
        _ = requireElement("request-title")
        pull(travel: 240, hold: 0.05)
        pageScroll().swipeUp(velocity: .slow)
        waitForCards(atLeast: 8, timeout: 12)
        Thread.sleep(forTimeInterval: 1.2)
        XCTAssertEqual(pullState(), "idle")
        XCTAssertEqual(cards().count, 8)
        assertTheRequestDidNotMoveWhenTheThreadCameIn(readProbe("touch"))
    }


    /// Not a check: a slow pull held at its armed place, then a history that takes its time, so that a screenshot loop outside
    /// can catch every stage (the plan's R3 says how). The moments are written to a file; the probe has the rest.
    func testStoryboardForTheReport() throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["AH_UI_STORYBOARD"] == "1", "only for the screenshots of the report")
        launch(extra: ["-AHHistoryDelay", "6", "-AHPullProbe", probePath("story")])
        _ = requireElement("request-title")
        func mark(_ label: String) {
            let line = "\(CACurrentMediaTime()) \(Date().timeIntervalSince1970) \(label)\n"
            let path = "/tmp/ah-story-marks.txt"
            if let handle = FileHandle(forWritingAtPath: path) {
                handle.seekToEndOfFile()
                handle.write(Data(line.utf8))
                try? handle.close()
            } else {
                try? line.write(toFile: path, atomically: true, encoding: .utf8)
            }
        }
        try? FileManager.default.removeItem(atPath: "/tmp/ah-story-marks.txt")
        Thread.sleep(forTimeInterval: 4)
        mark("pull begins")
        let scroll = pageScroll()
        let height = scroll.frame.height
        let start = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
        let end = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2 + 300 / height))
        start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 3)
        mark("lifted")
        Thread.sleep(forTimeInterval: 12)
        mark("done")
    }

    /// The Mac does not answer: the room becomes the row that says so, and the request does not move.
    func testAPullThatGetsNoAnswerLeavesARowInTheRoomItHeld() {
        launch(extra: ["-AHHistoryFail", "1", "-AHPullProbe", probePath("failed")])
        let title = requireElement("request-title")
        let start = title.frame.minY
        pull(travel: 240)
        waitFor("the pull never took hold (state \(pullState()))", timeout: 5) { pullState() == "loading" }
        Thread.sleep(forTimeInterval: 0.9)
        let held = title.frame.minY
        XCTAssertEqual(held - start, heldRoom, accuracy: 3)
        let row = requireElement("session-thread-error", timeout: 10)
        Thread.sleep(forTimeInterval: 0.8)
        shot("failed-row")
        XCTAssertEqual(title.frame.minY, held, accuracy: 1, "the request moved when the row came")
        XCTAssertLessThanOrEqual(row.frame.maxY, held + 1, "the row is above the request")
        XCTAssertEqual(pullState(), "idle")
    }
}
