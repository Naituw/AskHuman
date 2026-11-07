import XCTest

/// The round "Annotations" button of the answer bar, the list of notes it opens, and where a row of that list leads
/// (docs/specs/message-annotations.md D18, docs/plans/message-annotations.md §7.3), over the mock's `annotations`
/// scenario. Notes are written the way a person writes them: press on a one-word paragraph, choose the entry of the
/// selection menu, type, add (`AskHumanWebUITests` has the writing itself).
///
/// The menus and buttons they look for are found by their English names, except in the one test that is about Chinese.
@MainActor
final class AskHumanAnnotationUITests: XCTestCase {
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
    private func launch(language: String = "en", extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", "annotations", "-AHLatency", "0.05", "-AHAutoAdvance", "NO", "-AHWebTimeout", "60",
            "-AppleLanguages", "(\(language))", "-AppleLocale", language == "en" ? "en_US" : "zh_CN",
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

    /// Polls until the condition holds, for at most `timeout` seconds.
    private func eventually(_ timeout: TimeInterval = 15, _ condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return condition()
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

    private func open(_ id: String) {
        requireElement("inbox-row-\(id)").tap()
        _ = requireElement("request-title")
    }

    private func back() {
        app.navigationBars.buttons.element(boundBy: 0).tap()
    }

    /// The one-word paragraph, whatever the page has put after it (the number of a note on it).
    private func word(_ text: String) -> XCUIElement {
        let pattern = NSRegularExpression.escapedPattern(for: text) + ".{0,3}"
        return app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", pattern)).firstMatch
    }

    /// On the screen and not behind anything: what a person could press.
    private func visible(_ element: XCUIElement) -> Bool { element.exists && element.isHittable }

    /// An entry of the selection menu (or of anything else that is up) by its title.
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

    private func waitForTheSheetToGo(file: StaticString = #filePath, line: UInt = #line) {
        let gone = NSPredicate(format: "exists == false")
        let wait = XCTNSPredicateExpectation(predicate: gone, object: element("annotation-field"))
        XCTAssertEqual(XCTWaiter().wait(for: [wait], timeout: 10), .completed, "the sheet is still up", file: file, line: line)
    }

    /// Writes a note on a one-word paragraph, as a person does.
    private func writeNote(on text: String, _ note: String, file: StaticString = #filePath, line: UInt = #line) {
        guard let add = selectWord(word(text), until: "Add annotation") else {
            XCTFail("the menu of \(text) has no entry for a note", file: file, line: line)
            return
        }
        add.tap()
        requireElement("annotation-field", timeout: 10, file: file, line: line).typeText(note)
        requireElement("annotation-commit", file: file, line: line).tap()
        waitForTheSheetToGo(file: file, line: line)
        // The page draws the note and tells the screen where it is.
        Thread.sleep(forTimeInterval: 0.8)
    }

    private func openTheList(file: StaticString = #filePath, line: UInt = #line) {
        requireElement("annotations-button", timeout: 10, file: file, line: line).tap()
        _ = requireElement("annotation-row-0", timeout: 10, file: file, line: line)
    }

    private func scrollToTop() {
        for _ in 0..<12 { app.swipeDown(velocity: .fast) }
        Thread.sleep(forTimeInterval: 0.5)
    }

    // MARK: the bar's button

    func testTheBarHasNoAnnotationsButtonUntilANoteIsWritten() {
        launch()
        open("req-notes")
        XCTAssertTrue(word("Redis").waitForExistence(timeout: 60), "the page is drawn")
        XCTAssertFalse(element("annotations-button").exists, "nothing is written yet")

        writeNote(on: "Redis", "Does it survive a restart?")
        let button = requireElement("annotations-button", timeout: 10)
        XCTAssertEqual(button.label, "Annotations · 1", "the button says how many notes there are")
        shot("50-bar-button")

        // Taking the note away takes the button away with it.
        let edit = selectWord(word("Redis"), until: "Edit annotation")
        XCTAssertNotNil(edit, "a word with a note offers to edit it")
        edit?.tap()
        requireElement("annotation-delete", timeout: 10).tap()
        waitForTheSheetToGo()
        XCTAssertTrue(gone("annotations-button"), "no notes, no button")
    }

    // MARK: the list

    func testTheListHasTheNotesInTheOrderOfTheirPassages() {
        launch()
        open("req-notes")
        XCTAssertTrue(word("Redis").waitForExistence(timeout: 60), "the page is drawn")
        // Written the other way round: the list follows the message, not the order of writing.
        writeNote(on: "Memcached", "Is it simpler to run?")
        writeNote(on: "Redis", "Does it survive a restart?")

        openTheList()
        let first = element("annotation-row-0"), second = element("annotation-row-1")
        XCTAssertTrue(second.exists, "two notes, two rows")
        XCTAssertTrue(first.label.contains("Redis") && first.label.contains("Does it survive a restart?"), "first: \(first.label)")
        XCTAssertTrue(second.label.contains("Memcached") && second.label.contains("Is it simpler to run?"), "second: \(second.label)")
        XCTAssertTrue(app.navigationBars["Annotations · 2"].exists, "the title counts the notes")
        shot("51-list")

        requireElement("annotation-list-done").tap()
        XCTAssertTrue(gone("annotation-row-0"), "Done puts the list away")
        XCTAssertEqual(element("annotations-button").label, "Annotations · 2")
    }

    /// A row leads back to its passage, which is out of sight from the top of a message taller than the screen.
    func testATapOnARowGoesBackToThePassage() {
        launch()
        open("req-notes-tall")
        XCTAssertTrue(word("Alpha").waitForExistence(timeout: 60), "the page is drawn")

        // The last paragraph is a long way down: go there, write on it, and come back to the top.
        var swipes = 0
        while !visible(word("Omega")), swipes < 18 {
            app.swipeUp(velocity: .slow)
            swipes += 1
        }
        XCTAssertTrue(visible(word("Omega")), "the last paragraph is within reach after \(swipes) swipes")
        writeNote(on: "Omega", "Is this really the end?")
        scrollToTop()
        XCTAssertFalse(visible(word("Omega")), "from the top the passage is out of sight")
        shot("52-passage-out-of-sight")

        openTheList()
        element("annotation-row-0").tap()
        XCTAssertTrue(gone("annotation-row-0"), "the list leaves")
        XCTAssertTrue(eventually(20) { visible(word("Omega")) }, "the screen scrolled to the passage")
        // Not at the very top (it was not there) and not under the bar: in the upper part of what can be seen.
        let frame = word("Omega").frame
        let window = app.windows.firstMatch.frame
        XCTAssertGreaterThan(frame.minY, 80, "the passage is below the navigation bar")
        XCTAssertLessThan(frame.midY, window.height * 0.65, "and well clear of the bar at the bottom")
        shot("53-passage-revealed")
    }

    func testSwipingARowDeletesTheNoteAndEditingGoesToTheEditor() {
        launch()
        open("req-notes")
        XCTAssertTrue(word("Redis").waitForExistence(timeout: 60), "the page is drawn")
        writeNote(on: "Redis", "Does it survive a restart?")
        writeNote(on: "Memcached", "Is it simpler to run?")

        openTheList()
        element("annotation-row-0").swipeLeft()
        let delete = app.buttons["Delete"]
        XCTAssertTrue(delete.waitForExistence(timeout: 5), "a swipe to the left offers Delete")
        XCTAssertTrue(app.buttons["Edit"].exists, "and Edit")
        shot("54-swipe-actions")
        delete.tap()
        XCTAssertTrue(app.navigationBars["Annotations · 1"].waitForExistence(timeout: 5), "one note is left")
        XCTAssertTrue(element("annotation-row-0").label.contains("Memcached"), "and it is the other one")
        XCTAssertFalse(element("annotation-row-1").exists)

        // Edit: the list goes, the editor comes, on that note.
        element("annotation-row-0").swipeLeft()
        let edit = app.buttons["Edit"]
        XCTAssertTrue(edit.waitForExistence(timeout: 5))
        edit.tap()
        let field = requireElement("annotation-field", timeout: 15)
        XCTAssertEqual(field.value as? String, "Is it simpler to run?", "the editor has the note as it was written")
        field.tap()
        field.typeText(" And to monitor?")
        requireElement("annotation-commit").tap()
        waitForTheSheetToGo()
        XCTAssertFalse(element("annotation-row-0").exists, "the list does not come back by itself")
        XCTAssertEqual(element("annotations-button").label, "Annotations · 1")

        openTheList()
        XCTAssertTrue(element("annotation-row-0").label.contains("Is it simpler to run? And to monitor?"), "the change was kept")
    }

    func testDeletingTheLastNoteFromTheListClosesIt() {
        launch()
        open("req-notes")
        XCTAssertTrue(word("Redis").waitForExistence(timeout: 60), "the page is drawn")
        writeNote(on: "Redis", "Does it survive a restart?")

        openTheList()
        element("annotation-row-0").swipeLeft()
        let delete = app.buttons["Delete"]
        XCTAssertTrue(delete.waitForExistence(timeout: 5))
        delete.tap()
        XCTAssertTrue(gone("annotation-row-0"), "nothing is left to list")
        XCTAssertTrue(gone("annotation-list-done"), "the sheet went with the last note")
        XCTAssertTrue(gone("annotations-button"), "and so did the button")
    }

    // MARK: staying with the form

    func testNotesStayWhenTheRequestIsLeftAndOpenedAgain() {
        launch()
        open("req-notes")
        XCTAssertTrue(word("Redis").waitForExistence(timeout: 60), "the page is drawn")
        writeNote(on: "Redis", "Does it survive a restart?")

        back()
        requireElement("inbox-row-req-notes").tap()
        let button = requireElement("annotations-button", timeout: 20)
        XCTAssertEqual(button.label, "Annotations · 1", "the note is part of the draft")
        // The page draws it again too: the word offers to edit it.
        XCTAssertTrue(word("Redis").waitForExistence(timeout: 60), "the page is drawn again")
        XCTAssertNotNil(selectWord(word("Redis"), until: "Edit annotation"), "the passage is a note again")
        app.navigationBars.firstMatch.tap()
    }

    /// A message the phone cannot select in (drawn as plain text) offers no way to write a note, but the notes that are
    /// in the form stay, are listed, and are sent: the way to them is still there.
    func testNotesOfAMessageDrawnAsPlainTextStayReachable() {
        launch(extra: ["-AHOpen", "req-notes-plain", "-AHNotes", "1"])
        _ = requireElement("request-title")
        let button = requireElement("annotations-button", timeout: 30)
        XCTAssertEqual(button.label, "Annotations · 1")
        button.tap()
        let row = requireElement("annotation-row-0", timeout: 10)
        XCTAssertTrue(row.label.contains("Note 1"), row.label)
        shot("55-list-of-a-plain-message")
        // There is no passage to go to on a page that is not there: the tap only closes the list.
        row.tap()
        XCTAssertTrue(gone("annotation-row-0"), "the list leaves")
        XCTAssertTrue(element("annotations-button").exists, "the notes are still in the form")
    }

    // MARK: sending

    func testTheNotesAreInTheHistoryOfTheAnsweredRequest() {
        launch()
        open("req-notes")
        XCTAssertTrue(word("Redis").waitForExistence(timeout: 60), "the page is drawn")
        writeNote(on: "Redis", "Does it survive a restart?")

        requireElement("option-0-0").tap()
        let send = requireElement("send-button", timeout: 15)
        XCTAssertTrue(send.isEnabled, "the notes do not stop an answer")
        send.tap()
        // After a send the app goes back to the list by itself (when the finish overtakes the "sending" state of the mock the
        // page stays on its banner, and the way back is the button).
        if !app.tabBars.firstMatch.waitForExistence(timeout: 8) { back() }
        XCTAssertTrue(app.tabBars.firstMatch.waitForExistence(timeout: 10), "back at the list")

        app.tabBars.buttons["History"].tap()
        requireElement("history-row-req-notes").tap()
        XCTAssertTrue(app.staticTexts["Annotations · 1"].waitForExistence(timeout: 10), "the note section is there")
        XCTAssertTrue(app.staticTexts["Does it survive a restart?"].exists, "with what was written")
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "Redis")).firstMatch.exists, "and the passage")
        shot("56-history")
    }

    // MARK: Chinese

    func testTheListSpeaksChinese() {
        launch(language: "zh-Hans", extra: ["-AHOpen", "req-notes-plain", "-AHNotes", "2"])
        _ = requireElement("request-title")
        let button = requireElement("annotations-button", timeout: 30)
        XCTAssertEqual(button.label, "附注 · 2 条")
        button.tap()
        _ = requireElement("annotation-row-1", timeout: 10)
        XCTAssertTrue(app.navigationBars["附注 · 2 条"].exists, "the title is in Chinese")
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "左滑可编辑或删除")).firstMatch.exists, "so is the hint")
        shot("57-list-zh")
    }
}
