import XCTest

/// A request that is one question and nothing else shows the question in the message area, above the options, where a
/// message would be (docs/specs/message-annotations.md D17, docs/plans/message-annotations.md §8), over the mock's `direct`
/// scenario. What is told apart on the screen is the space between the question and the first option: inside one card
/// it is the 12 points between a card's rows, and between two cards it is the card's two paddings and the gap between
/// them, which is four times as much.
@MainActor
final class AskHumanDirectAskUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        if let run = testRun, run.totalFailureCount > 0, app != nil { shot("failed-" + name.filter { $0.isLetter }) }
    }

    // MARK: helpers

    /// `webTimeout` is long because the simulator takes seconds to start web content the first time.
    @discardableResult
    private func launch(open id: String, extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", "direct", "-AHLatency", "0.05", "-AHAutoAdvance", "NO", "-AHWebTimeout", "60",
            "-AHOpen", id, "-AppleLanguages", "(en)", "-AppleLocale", "en_US",
        ] + extra
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

    private func gone(_ id: String, timeout: TimeInterval = 10) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element(id))
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"], !folder.isEmpty {
            let url = URL(fileURLWithPath: folder).appendingPathComponent("\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
            return
        }
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func back() {
        app.navigationBars.buttons.element(boundBy: 0).tap()
    }

    /// The text that has `fragment` in it, wherever it is drawn (the page, or the native text of a card).
    private func text(_ fragment: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", fragment)).firstMatch
    }

    /// The one-word paragraph, whatever the page has put after it (the number of a note on it).
    private func word(_ text: String) -> XCUIElement {
        let pattern = NSRegularExpression.escapedPattern(for: text) + ".{0,3}"
        return app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", pattern)).firstMatch
    }

    /// The space between the bottom of the question and the top of the first option.
    private func space(below question: String, file: StaticString = #filePath, line: UInt = #line) -> CGFloat {
        let text = self.text(question)
        XCTAssertTrue(text.waitForExistence(timeout: 60), "the question \"\(question)\" is drawn", file: file, line: line)
        let option = requireElement("option-0-0", file: file, line: line)
        return option.frame.minY - text.frame.maxY
    }

    /// What a card's two paddings and the space between two cards make: more than any gap inside a card.
    private let betweenCards: CGFloat = 30

    private func entry(_ title: String) -> XCUIElement {
        for query in [app.menuItems, app.buttons, app.staticTexts, app.otherElements] {
            let found = query[title]
            if found.exists { return found }
        }
        return app.menuItems[title]
    }

    private func pollFor(_ title: String, seconds: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            let found = entry(title)
            if found.exists { return found }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return nil
    }

    /// Presses on a word until the menu of its selection shows `title` (see `AskHumanWebUITests.selectWord`).
    private func selectWord(_ word: XCUIElement, until title: String) -> XCUIElement? {
        for _ in 1...3 {
            word.press(forDuration: 1.2)
            if let found = pollFor(title, seconds: 8) { return found }
            word.tap()
            if let found = pollFor(title, seconds: 8) { return found }
            app.navigationBars.firstMatch.tap()
            Thread.sleep(forTimeInterval: 0.5)
        }
        return nil
    }

    // MARK: one question and nothing else

    func testAShortQuestionIsInTheMessageAreaAboveTheOptions() {
        launch(open: "req-direct-short")
        _ = requireElement("request-title")
        let gap = space(below: "Should I carry on deploying to staging?")
        XCTAssertGreaterThan(gap, betweenCards, "the question is in the message card and the options are in the next one: \(gap)")
        XCTAssertTrue(requireElement("option-0-1").exists, "the options are all there")
        shot("60-direct-short")
    }

    func testALongQuestionTakesNotesLikeAMessage() {
        launch(open: "req-direct-long")
        _ = requireElement("request-title")
        XCTAssertTrue(word("Redis").waitForExistence(timeout: 60), "the question is drawn as a message page")
        XCTAssertFalse(element("annotations-button").exists, "nothing is written yet")

        guard let add = selectWord(word("Redis"), until: "Add annotation") else {
            XCTFail("the question has no entry for a note in its selection menu")
            return
        }
        add.tap()
        requireElement("annotation-field", timeout: 10).typeText("Does it survive a restart?")
        requireElement("annotation-commit").tap()
        XCTAssertTrue(gone("annotation-field"), "the sheet went")
        let button = requireElement("annotations-button", timeout: 10)
        XCTAssertEqual(button.label, "Annotations · 1")
        shot("61-direct-long-with-a-note")
    }

    func testAStrictChoiceHasItsQuestionAboveToo() {
        launch(open: "req-direct-strict")
        _ = requireElement("request-title")
        let gap = space(below: "Which environment should the release go to?")
        XCTAssertGreaterThan(gap, betweenCards, "strict or not, a lone question goes to the message area: \(gap)")
        XCTAssertFalse(element("annotations-button").exists, "a strict choice takes no notes")
        shot("62-direct-strict")
    }

    func testPlainTextHasItsQuestionAboveToo() {
        launch(open: "req-direct-plain")
        _ = requireElement("request-title")
        let gap = space(below: "Plain text, no Markdown: may I carry on?")
        XCTAssertGreaterThan(gap, betweenCards, "drawn natively there, with no way to write a note: \(gap)")
        shot("63-direct-plain")
    }

    func testAMacWithoutNotesHasItsQuestionAboveToo() {
        launch(open: "req-direct-old")
        _ = requireElement("request-title")
        let gap = space(below: "Can I carry on from here?")
        XCTAssertGreaterThan(gap, betweenCards, "the place of the question does not depend on the Mac: \(gap)")
        XCTAssertFalse(element("annotations-button").exists)
        shot("64-direct-old-mac")
    }

    // MARK: everything else keeps its cards

    func testWhatsNextKeepsItsQuestionInItsCard() {
        launch(open: "req-direct-next")
        _ = requireElement("request-title")
        let gap = space(below: "What should I do next?")
        XCTAssertLessThan(gap, betweenCards, "What's next is not moved: \(gap)")
        shot("65-direct-whats-next")
    }

    func testTwoQuestionsKeepTheirCards() {
        launch(open: "req-direct-two")
        _ = requireElement("request-title")
        let gap = space(below: "Which branch should I use?")
        XCTAssertLessThan(gap, betweenCards, "two questions are not moved: \(gap)")
        XCTAssertTrue(text("Which target should I build?").exists, "the second question is there too")
        shot("66-direct-two")
    }

    func testAMessageKeepsTheQuestionInItsCard() {
        launch(open: "req-direct-message")
        _ = requireElement("request-title")
        XCTAssertTrue(text("Staging is healthy").waitForExistence(timeout: 60), "the message is there")
        let gap = space(below: "Deploy to production now?")
        XCTAssertLessThan(gap, betweenCards, "a message and a question are two things, as before: \(gap)")
        shot("67-direct-message")
    }

    // MARK: answering

    func testAnAnswerToAMovedQuestionIsSentAsEver() {
        launch(open: "req-direct-short")
        _ = requireElement("request-title")
        requireElement("option-0-0", timeout: 30).tap()
        let send = requireElement("send-button", timeout: 15)
        XCTAssertTrue(send.isEnabled)
        send.tap()
        // After a send the app goes back to the list by itself (when the finish overtakes the "sending" state of the mock the
        // page stays on its banner, and the way back is the button).
        if !element("filter-menu").waitForExistence(timeout: 8) { back() }
        XCTAssertTrue(element("filter-menu").waitForExistence(timeout: 10), "back at the list")
        app.tabBars.buttons["History"].tap()
        XCTAssertTrue(element("history-row-req-direct-short").waitForExistence(timeout: 10), "the request is in the history")
    }
}
