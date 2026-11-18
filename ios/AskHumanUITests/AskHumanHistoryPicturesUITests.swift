import XCTest

/// The pictures in the history (`docs/specs/ios-history-pictures.md`), through the real UI over the mock Macs' `historyPictures`
/// scenario, in English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`).
///
/// The request `req-pictures-now` has a session of five exchanges, oldest first on the page: `ex-pics-01` (two pictures in the
/// answer), `ex-pics-02` (two pictures, both gone), `ex-pics-03` (two, one of them gone), `ex-pics-04` (one) and `ex-pics-05`
/// (seven, and a file that is not a picture). The History tab has the finished request `ex-pics-05`.
@MainActor
final class AskHumanHistoryPicturesUITests: XCTestCase {
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
        open request: String? = "req-pictures-now", extra: [String] = [], contentSize: String? = nil, dark: Bool = false
    ) -> XCUIApplication {
        app?.terminate()
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        var arguments = [
            "-AHMock", "YES", "-AHLatency", "0.05", "-AHScenario", "historyPictures", "-AHWebMarkdown", "NO",
            "-AHAutoAdvance", "NO", "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ] + extra
        if let request { arguments += ["-AHOpen", request] }
        if let contentSize { arguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategory" + contentSize] }
        if dark {
            XCUIDevice.shared.appearance = .dark
            addTeardownBlock { XCUIDevice.shared.appearance = .light }
        }
        app.launchArguments = arguments
        app.launch()
        self.app = app
        if request != nil { _ = requireElement("request-menu") }
        return app
    }

    private func localized(_ en: String, _ zh: String) -> String { language == "en" ? en : zh }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// The first element whose identifier begins with `prefix` (a picture's identifier ends with its asset id).
    private func element(prefix: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", prefix)).firstMatch
    }

    /// A slow drag inside `element`, from a height to another one (fractions of its height, 0 is its top).
    private func drag(_ element: XCUIElement, from: CGFloat, to: CGFloat) {
        let start = element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: from))
        let end = element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: to))
        start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.3)
    }

    /// Drags until the picture does not move any more (it has come to the end of its travel in that direction).
    private func dragUntilItStops(_ element: XCUIElement, _ picture: XCUIElement, from: CGFloat, to: CGFloat) {
        var last = picture.frame.minY
        for _ in 0..<8 {
            drag(element, from: from, to: to)
            Thread.sleep(forTimeInterval: 0.5)
            let now = picture.frame.minY
            if abs(now - last) < 0.5 { return }
            last = now
        }
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
            let url = URL(fileURLWithPath: folder).appendingPathComponent("pictures-\(language)-\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
            return
        }
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
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

    private func settle() { Thread.sleep(forTimeInterval: 1.2) }

    /// The history of the page, loaded from the menu: all five exchanges are one page.
    private func openThread() {
        requireElement("request-menu").tap()
        let item = element("request-menu-session-history")
        XCTAssertTrue(item.waitForExistence(timeout: 5), "the menu has no session history item")
        item.tap()
        waitFor("fewer than five cards appeared (\(cards().count))") { cards().count >= 5 }
        settle()
    }

    /// The card of an exchange, counted from the oldest (`0` is `ex-pics-01`, `4` is `ex-pics-05`).
    private func card(_ index: Int) -> XCUIElement { cards().element(boundBy: index) }

    /// Drags the page down a little at a time until `target` is on screen (the cards above the newest are off it).
    private func reveal(_ target: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        let scroll = app.scrollViews.allElementsBoundByIndex.max { $0.frame.height < $1.frame.height } ?? app.scrollViews.firstMatch
        let window = app.windows.firstMatch.frame
        for _ in 0..<12 {
            if target.exists, target.frame.minY > 110, target.frame.maxY < window.maxY - 60, target.isHittable { return }
            let start = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
            let end = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.55))
            start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .default, thenHoldForDuration: 0.2)
        }
        XCTAssertTrue(target.exists && target.isHittable, "could not bring \(target) on screen", file: file, line: line)
    }

    /// The part of a card that reads its head out and opens it (a card with pictures is read in pieces).
    private func head(of card: XCUIElement, containing text: String) -> XCUIElement {
        card.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    private func closeViewer() {
        requireElement("gallery-close").tap()
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: element("image-gallery"))
        waitForExpectations(timeout: 5)
    }

    // MARK: the folded row

    func testAFoldedCardShowsAtMostFourPicturesAndCountsTheRest() {
        launch()
        openThread()
        let newest = card(4)
        for index in 0..<4 {
            XCTAssertTrue(newest.buttons["session-picture-\(index)"].waitForExistence(timeout: 5), "picture \(index) is in the row")
        }
        XCTAssertFalse(newest.buttons["session-picture-4"].exists, "no more than four cells")
        let last = newest.buttons["session-picture-3"].label
        XCTAssertTrue(last.contains("onboarding-4.png"), last)
        XCTAssertTrue(last.contains("3"), "the last cell says three more follow it: \(last)")
        XCTAssertFalse(newest.buttons["session-picture-grid-0"].exists, "the grid is for an open card")
        shot("folded-row")
    }

    func testAnExchangeWithOnePictureHasOneCell() {
        launch()
        openThread()
        let one = card(3)
        reveal(one)
        XCTAssertTrue(one.buttons["session-picture-0"].exists)
        XCTAssertFalse(one.buttons["session-picture-1"].exists)
    }

    // MARK: the viewer

    func testATapOnAPictureOpensTheViewerOnItAndDoesNotOpenTheCard() {
        launch()
        openThread()
        let newest = card(4)
        let head = head(of: newest, containing: "Seven screenshots")
        XCTAssertEqual(head.value as? String, localized("Collapsed", "已折叠"))
        newest.buttons["session-picture-1"].tap()
        requireElement("image-gallery")
        XCTAssertEqual(element("gallery-name").label, "onboarding-2.png", "the viewer opens on the picture that was tapped")
        let position = element("gallery-position").label
        XCTAssertTrue(position.contains("2") && position.contains("7"), "the second of seven: \(position)")
        shot("viewer")
        closeViewer()
        XCTAssertEqual(head.value as? String, localized("Collapsed", "已折叠"), "the card did not open")
    }

    /// `onboarding-2.png` is a phone screenshot, taller than the viewer's sheet. Laid out over the whole sheet its top is behind
    /// the bar (the close and share buttons, the heading) and, on a phone, near the island; it has to start below them.
    func testATallPictureStartsBelowTheBarOfTheViewer() {
        launch()
        openThread()
        card(4).buttons["session-picture-1"].tap()
        requireElement("image-gallery")
        XCTAssertEqual(element("gallery-name").label, "onboarding-2.png")
        let picture = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'viewer-picture-hpic-2-'")).firstMatch
        XCTAssertTrue(picture.waitForExistence(timeout: 20), "the picture did not arrive")
        waitFor("the picture is not in place") { picture.frame.height > 100 }
        settle()
        let bar = max(element("gallery-close").frame.maxY, element("gallery-name").frame.maxY)
        let window = app.windows.firstMatch.frame
        let frame = picture.frame
        XCTAssertGreaterThanOrEqual(frame.minY, bar - 0.5, "the picture \(frame) starts above the bottom of the bar (\(bar))")
        XCTAssertLessThanOrEqual(frame.maxY, window.maxY + 0.5, "the picture \(frame) is within the screen \(window)")
        XCTAssertGreaterThanOrEqual(frame.minX, window.minX - 0.5)
        XCTAssertLessThanOrEqual(frame.maxX, window.maxX + 0.5)
        shot("viewer-tall")
        closeViewer()
    }

    /// Zoomed in, a picture is not cut off at the bar: the area it moves in is the whole sheet, so it can be dragged out
    /// under the bar (as in the system's Photos app), where the system softens or blurs it. Its edges still stop where
    /// they rested: dragged all the way down, its top comes back to the bar's lower edge; dragged all the way up, its
    /// bottom is at the bottom of the sheet. (Before, the area began at the bar's lower edge, so the picture was cut
    /// off there and the bar was an empty strip.)
    func testAZoomedPictureCanBeDraggedUnderTheBarOfTheViewer() {
        launch()
        openThread()
        card(4).buttons["session-picture-1"].tap()
        requireElement("image-gallery")
        XCTAssertEqual(element("gallery-name").label, "onboarding-2.png")
        let picture = element(prefix: "viewer-picture-hpic-2-")
        let area = element(prefix: "viewer-zoom-hpic-2-")
        XCTAssertTrue(picture.waitForExistence(timeout: 20), "the picture did not arrive")
        XCTAssertTrue(area.exists, "the area the picture moves in is not there")
        waitFor("the picture is not in place") { picture.frame.height > 100 }
        settle()
        let close = element("gallery-close").frame
        let bar = max(close.maxY, element("gallery-name").frame.maxY)
        // The sheet is not the window: an iPad shows it as a form sheet in the middle of the screen.
        let sheet = element("image-gallery").frame
        let rest = picture.frame
        // At rest the picture is below the bar, but the area it moves in runs up under it.
        XCTAssertGreaterThanOrEqual(rest.minY, bar - 0.5, "the picture \(rest) rests above the bottom of the bar (\(bar))")
        XCTAssertLessThanOrEqual(area.frame.minY, close.minY, "the area \(area.frame) begins below the top of the bar's buttons \(close)")
        XCTAssertEqual(area.frame.maxY, sheet.maxY, accuracy: 1, "the area \(area.frame) does not run to the bottom of the sheet \(sheet)")
        shot("viewer-zoom-1-at-rest")

        area.doubleTap()
        waitFor("the picture did not zoom in") { picture.frame.height > rest.height * 2 }
        settle()
        shot("viewer-zoom-2-zoomed-in")
        // Pulled up by a good part of the sheet, the glyph in the middle of the picture passes under the bar.
        drag(area, from: 0.5, to: 0.0)
        settle()
        XCTAssertLessThan(picture.frame.minY, rest.minY - 100, "the picture \(picture.frame) did not move up")
        shot("viewer-zoom-3-under-the-bar")

        // All the way down: the top of the picture rests at the bar's lower edge again, neither under it nor below it.
        dragUntilItStops(area, picture, from: 0.15, to: 0.95)
        XCTAssertEqual(picture.frame.minY, rest.minY, accuracy: 1, "the top of the picture \(picture.frame) is not at the bar's lower edge \(rest.minY)")
        shot("viewer-zoom-4-top")
        // All the way up: the bottom of the picture is at the bottom of the sheet.
        dragUntilItStops(area, picture, from: 0.95, to: 0.15)
        XCTAssertEqual(picture.frame.maxY, area.frame.maxY, accuracy: 1, "the bottom of the picture \(picture.frame) is not at the bottom of the area \(area.frame)")
        shot("viewer-zoom-5-bottom")

        // A second double tap brings the picture back to where it rested.
        area.doubleTap()
        waitFor("the picture did not come back") { abs(picture.frame.height - rest.height) < 1 }
        settle()
        let back = picture.frame
        XCTAssertEqual(back.minY, rest.minY, accuracy: 1)
        XCTAssertEqual(back.minX, rest.minX, accuracy: 1)
        XCTAssertEqual(back.width, rest.width, accuracy: 1)
        closeViewer()
    }

    func testTheViewerPagesThroughThePicturesOfTheExchange() {
        launch()
        openThread()
        card(4).buttons["session-picture-0"].tap()
        requireElement("image-gallery")
        let gallery = element("image-gallery")
        gallery.swipeLeft()
        waitFor("the viewer did not page to the next picture") { self.element("gallery-name").label == "onboarding-2.png" }
        closeViewer()
    }

    // MARK: an open card

    func testAnOpenCardHasAGridAndCountsOnlyTheFilesThatAreNotPictures() {
        launch()
        openThread()
        let newest = card(4)
        head(of: newest, containing: "Seven screenshots").tap()
        for index in 0..<7 {
            XCTAssertTrue(newest.buttons["session-picture-grid-\(index)"].waitForExistence(timeout: 5), "cell \(index) of the grid")
        }
        XCTAssertFalse(newest.buttons["session-picture-0"].exists, "the row is for a folded card")
        XCTAssertTrue(
            newest.staticTexts[localized("1 attachment", "附件 1 个")].exists,
            "eight files were attached and seven of them are the pictures above"
        )
        shot("open-grid")
        newest.buttons["session-picture-grid-4"].tap()
        requireElement("image-gallery")
        XCTAssertEqual(element("gallery-name").label, "onboarding-5.png")
        closeViewer()
        XCTAssertTrue(newest.buttons["session-picture-grid-0"].exists, "the card is still open")
    }

    func testThePicturesOfAnAnswerAreUnderTheAnswerWhenTheCardIsOpen() {
        launch()
        openThread()
        let oldest = card(0)
        reveal(oldest)
        // Folded, the row has what the exchange has: the two pictures of the answer.
        XCTAssertTrue(oldest.buttons["session-picture-0"].exists)
        XCTAssertTrue(oldest.buttons["session-picture-1"].exists)
        head(of: oldest, containing: "header look").tap()
        XCTAssertTrue(oldest.buttons["session-picture-grid-0"].waitForExistence(timeout: 5))
        XCTAssertTrue(oldest.buttons["session-picture-grid-1"].exists)
        oldest.buttons["session-picture-grid-1"].tap()
        requireElement("image-gallery")
        XCTAssertEqual(element("gallery-name").label, "reply-2.png")
        XCTAssertTrue(element("gallery-position").label.contains("2"), "the second of the two")
        closeViewer()
    }

    // MARK: pictures that are gone

    func testAPictureWhoseFileIsGoneIsAPlaceAndTappingItSaysWhy() {
        launch()
        openThread()
        let partly = card(2)
        reveal(partly)
        XCTAssertTrue(partly.buttons["session-picture-0"].exists, "the one that is still there")
        let place = partly.buttons["session-picture-gone-1"]
        XCTAssertTrue(place.exists, "the one that is gone is a place held")
        XCTAssertTrue(place.label.contains(localized("no longer on the Mac", "已不在 Mac 上")), place.label)
        shot("gone-place")
        place.tap()
        let alert = app.alerts.firstMatch
        XCTAssertTrue(alert.waitForExistence(timeout: 5), "a note says why there is nothing to open")
        XCTAssertTrue(alert.staticTexts[localized("No longer on the Mac", "已不在 Mac 上")].exists)
        XCTAssertFalse(element("image-gallery").exists, "there is no viewer for it")
        alert.buttons.firstMatch.tap()
        waitFor("the note stayed") { !self.app.alerts.firstMatch.exists }
    }

    func testAnExchangeWhosePicturesAreAllGoneKeepsItsPlaces() {
        launch()
        openThread()
        let none = card(1)
        reveal(none)
        XCTAssertTrue(none.buttons["session-picture-gone-0"].exists)
        XCTAssertTrue(none.buttons["session-picture-gone-1"].exists)
    }

    // MARK: text size, appearance

    func testTheRowStaysWithinTheCardAtTheLargestTextSize() throws {
        // At this size an iPad's toolbar folds the request menu into the system "More" button, which has no identifier. The
        // iPhone is the narrower layout (the row wraps there), so it is where the row is checked.
        try XCTSkipIf(UIDevice.current.userInterfaceIdiom == .pad, "The request menu is folded into the system overflow button on an iPad at this text size.")
        launch(contentSize: "AccessibilityXXXL")
        openThread()
        let newest = card(4)
        for index in 0..<4 {
            XCTAssertTrue(newest.buttons["session-picture-\(index)"].waitForExistence(timeout: 5), "picture \(index) is in the row")
        }
        let frame = newest.frame, window = app.windows.firstMatch.frame
        for index in 0..<4 {
            let cell = newest.buttons["session-picture-\(index)"].frame
            XCTAssertTrue(frame.contains(cell), "cell \(index) \(cell) is within the card \(frame)")
            XCTAssertLessThanOrEqual(cell.maxX, window.maxX + 0.5, "cell \(index) is within the screen")
        }
        shot("large-text")
    }

    func testTheFoldedRowInTheDarkLooksRight() {
        launch(dark: true)
        openThread()
        XCTAssertTrue(card(4).buttons["session-picture-0"].waitForExistence(timeout: 5))
        reveal(card(2))
        shot("dark-row")
    }

    // MARK: the History tab

    func testTheDetailPageOfAFinishedRequestGetsItsPictures() {
        // The tab is picked by launch argument: an iPad's top tab bar is not a `tabBars` element.
        launch(open: nil, extra: ["-AHTab", "history"])
        requireElement("history-row-ex-pics-05").tap()
        // The page is whole first; the pictures take the place of the names when they come.
        for index in 0..<7 {
            XCTAssertTrue(element("history-picture-m-grid-\(index)").waitForExistence(timeout: 15), "picture \(index) of the message")
        }
        XCTAssertFalse(element("history-picture-m-grid-7").exists)
        XCTAssertTrue(app.staticTexts["checklist.md"].exists, "a file that is not a picture stays a name")
        XCTAssertFalse(app.staticTexts["onboarding-1.png"].exists, "the pictures took the place of their names")
        shot("history-detail")
        element("history-picture-m-grid-2").tap()
        requireElement("image-gallery")
        XCTAssertEqual(element("gallery-name").label, "onboarding-3.png")
        closeViewer()
    }

    func testARecordTheMacDoesNotHaveLeavesThePageAsItWas() {
        launch(open: nil, extra: ["-AHTab", "history"])
        requireElement("history-row-req-old-1").tap()
        requireElement("history-title")
        Thread.sleep(forTimeInterval: 1.5)
        XCTAssertFalse(app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'history-picture-'")).firstMatch.exists)
        XCTAssertFalse(app.alerts.firstMatch.exists, "nothing to report")
    }
}
