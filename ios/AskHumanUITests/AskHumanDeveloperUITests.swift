import XCTest

/// The developer mode (`docs/specs/ios-developer-mode.md`) through the real UI over the mock backend, in
/// English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`): the floating button, its menu, and what a finger can
/// do to them. The button lives in a window of its own, so these also check that it sits above the others and
/// lets the touches that are not its own through.
@MainActor
final class AskHumanDeveloperUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    /// The "AskHuman" project of the demo's iCloud source (the first row of the Todos tab).
    private let askHuman = "a1b2c3d4e5f60001"

    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        // What the screen looked like when a test failed.
        if let run = testRun, run.totalFailureCount > 0, app != nil { shot("failed-" + name.filter { $0.isLetter }) }
    }

    // MARK: helpers

    @discardableResult
    private func launch(mode: Bool = true, scenario: String = "demo", extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", scenario, "-AHLatency", "0.05", "-AHAutoAdvance", "NO",
            "-AHDeveloperMode", mode ? "YES" : "NO", "-AHTab", "todos",
            "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
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

    /// Polls until `condition` holds (an animation takes a moment to arrive).
    private func eventually(timeout: TimeInterval = 5, _ condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return true }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        }
        return condition()
    }

    private func localized(_ en: String, _ zh: String) -> String { language == "en" ? en : zh }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"], !folder.isEmpty {
            let url = URL(fileURLWithPath: folder).appendingPathComponent("\(language)-\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
            return
        }
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private var button: XCUIElement { element("dev-button") }

    /// A button by its identifier (the text inside a bar button carries the identifier too, and is not
    /// the one that is enabled or not).
    private func control(_ id: String) -> XCUIElement {
        app.buttons.matching(identifier: id).firstMatch
    }

    /// A keyboard that is shown for the first time on a simulator offers sliding to type first.
    private func dismissTheKeyboardTip() {
        let proceed = app.buttons[localized("Continue", "继续")]
        if proceed.waitForExistence(timeout: 2) { proceed.tap() }
    }

    /// Puts the cursor in the search field of the project list. The sheet is still sliding in when the list is
    /// there, and a tap on a field that is moving lands somewhere else (on the dimmed part of the screen, which
    /// closes the page), so the field is waited for until it holds still.
    private func focusTheSearch(file: StaticString = #filePath, line: UInt = #line) {
        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 10), "the list has no search field", file: file, line: line)
        var last = CGRect.null
        var stableSince: Date?
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            let frame = field.frame
            if frame == last {
                if let since = stableSince, Date().timeIntervalSince(since) > 0.6 { break }
                if stableSince == nil { stableSince = Date() }
            } else {
                last = frame
                stableSince = nil
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.2))
        }
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), "the search field took no keyboard", file: file, line: line)
        dismissTheKeyboardTip()
    }

    private func typeIntoTheField(_ text: String) {
        let field = requireElement("quick-todo-field")
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10), "the page opens with the keyboard up")
        dismissTheKeyboardTip()
        field.typeText(text)
    }
    private var row: XCUIElement { element("dev-menu-quickTodo") }

    private var origin: XCUICoordinate { app.coordinate(withNormalizedOffset: .zero) }

    private func point(_ x: CGFloat, _ y: CGFloat) -> XCUICoordinate {
        origin.withOffset(CGVector(dx: x, dy: y))
    }

    private var screen: CGRect { app.windows.firstMatch.frame }

    /// A finger goes down on the middle of `from`, stays `hold` seconds, moves to `target` (in points of the
    /// screen) and lets go.
    private func slide(from: XCUIElement, hold: TimeInterval, to target: CGPoint) {
        from.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: hold, thenDragTo: point(target.x, target.y))
    }

    /// Where the one row of the menu is when it opens from `button`: toward the inside of the screen, level
    /// with it (the menu is 220 pt wide at the least, 10 pt from the button).
    private func rowCenter(opensFrom button: XCUIElement) -> CGPoint {
        let frame = button.frame
        let toTheLeft = frame.midX >= screen.width / 2
        let dx: CGFloat = 22 + 10 + 110
        return CGPoint(x: toTheLeft ? frame.midX - dx : frame.midX + dx, y: frame.midY)
    }

    private func tab(_ index: Int) {
        let buttons = app.tabBars.buttons
        XCTAssertTrue(buttons.element(boundBy: index).waitForExistence(timeout: 10))
        buttons.element(boundBy: index).tap()
    }

    /// A lazy form only creates the rows near the visible area: scroll until `id` is clear of the bars.
    private func reveal(_ id: String) {
        var tries = 0
        while !element(id).exists, tries < 8 {
            app.swipeUp(velocity: .slow)
            tries += 1
        }
        let target = requireElement(id)
        for _ in 0..<10 {
            let frame = target.frame
            if frame.minY > 120, frame.maxY < screen.height - 220 { return }
            if frame.minY <= 120 { app.swipeDown(velocity: .slow) } else { app.swipeUp(velocity: .slow) }
        }
    }

    // MARK: the switch

    func testTheSwitchPutsTheButtonOnEveryScreenAndTakesItAway() {
        launch(mode: false, extra: ["-AHTab", "macs"])
        reveal("developer-mode-toggle")
        let toggle = requireElement("developer-mode-toggle")
        XCTAssertEqual(toggle.value as? String, "0", "the mode starts off")
        XCTAssertFalse(button.exists, "off, the app has no button")
        shot("dev-01-settings-off")

        toggle.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
        XCTAssertEqual(toggle.value as? String, "1")
        requireElement("dev-button", timeout: 5)
        shot("dev-02-settings-on")

        // It stays when the app goes to another screen.
        tab(2)
        XCTAssertTrue(button.exists, "the button is above every tab")
        tab(4)
        reveal("developer-mode-toggle")

        toggle.coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
        XCTAssertEqual(toggle.value as? String, "0")
        XCTAssertTrue(gone("dev-button", timeout: 5), "turned off, the button goes at once")
    }

    // MARK: the menu

    func testATapOpensTheMenuAndATapOutsideClosesItBeforeItReachesTheScreen() {
        launch()
        button.tap()
        _ = requireElement("dev-menu-quickTodo", timeout: 5)
        shot("dev-03-menu")

        // The menu opens toward the inside of the screen, level with the button, and stays.
        XCTAssertLessThan(row.frame.maxX, button.frame.minX)
        XCTAssertEqual(row.frame.midY, button.frame.midY, accuracy: 30)
        RunLoop.current.run(until: Date().addingTimeInterval(0.6))
        XCTAssertTrue(row.exists, "a tap leaves the menu open")

        // A tap outside closes it, and does not go on to the project under it.
        element("todo-project-" + askHuman).coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5)).tap()
        XCTAssertTrue(gone("dev-menu-quickTodo", timeout: 5), "a tap outside closes the menu")
        XCTAssertFalse(element("todo-project-screen").exists, "the tap that closed the menu was not the screen's")

        // The next one is the screen's.
        element("todo-project-" + askHuman).tap()
        _ = requireElement("todo-project-screen", timeout: 10)
        XCTAssertTrue(button.exists, "the button is above the project page too")
    }

    func testTouchesThatAreNotTheButtonsReachTheAppUnderIt() {
        launch()
        tab(4)
        XCTAssertTrue(app.tabBars.buttons.element(boundBy: 4).isSelected, "the tab bar takes touches with the button up")
        XCTAssertTrue(button.exists)
    }

    func testSlidingOffTheMenuLetsGoOfItAndNothingHappens() {
        launch()
        let before = button.frame
        slide(from: button, hold: 0.5, to: CGPoint(x: 60, y: 120))
        XCTAssertTrue(gone("dev-menu-quickTodo", timeout: 5), "let go outside the menu, it closes")
        XCTAssertEqual(button.frame.midX, before.midX, accuracy: 2, "the button did not move: the finger was choosing, not dragging")
        XCTAssertEqual(button.frame.midY, before.midY, accuracy: 2)
    }

    // MARK: dragging

    func testDraggingMovesTheButtonAndItSnapsToTheNearestEdge() {
        launch()
        let start = button.frame
        XCTAssertGreaterThan(start.midX, screen.width / 2, "it starts on the right")

        slide(from: button, hold: 0.01, to: CGPoint(x: 70, y: 600))
        XCTAssertTrue(eventually { self.button.frame.midX < 60 }, "dropped on the left half it goes to the left edge: \(button.frame)")
        XCTAssertEqual(button.frame.midY, 600, accuracy: 24, "and stays where it was dropped, up and down")
        XCTAssertFalse(element("dev-menu-quickTodo").exists, "dragging does not open the menu")
        shot("dev-04-dragged-left")

        // It keeps its place when the app moves on.
        tab(2)
        XCTAssertLessThan(button.frame.midX, 60)
        XCTAssertEqual(button.frame.midY, 600, accuracy: 24)

        // From the left the menu opens toward the right.
        button.tap()
        _ = requireElement("dev-menu-quickTodo", timeout: 5)
        XCTAssertGreaterThan(row.frame.minX, button.frame.maxX)
        shot("dev-05-menu-from-the-left")
    }

    func testTheButtonCannotBeDraggedOutOfTheSafeArea() {
        launch()
        slide(from: button, hold: 0.01, to: CGPoint(x: screen.width - 4, y: 2))
        XCTAssertTrue(eventually { self.button.frame.midY < 140 }, "it went up: \(button.frame)")
        XCTAssertGreaterThan(button.frame.minY, 40, "but not under the status bar: \(button.frame)")
        XCTAssertLessThan(button.frame.maxX, screen.width + 1)
        slide(from: button, hold: 0.01, to: CGPoint(x: 4, y: screen.height - 2))
        XCTAssertTrue(eventually { self.button.frame.midY > screen.height - 140 }, "it went down: \(button.frame)")
        XCTAssertLessThan(button.frame.maxY, screen.height - 20, "but not under the home indicator: \(button.frame)")
        XCTAssertGreaterThanOrEqual(button.frame.minX, 0)
    }

    // MARK: the quick-add page

    /// The same finger as a user's: down on the button, a moment's rest, then to the row, and up.
    private func openTheQuickAddPageWithOneFinger() {
        slide(from: button, hold: 0.5, to: rowCenter(opensFrom: button))
    }

    func testPressSlideAndLetGoOnTheRowOpensThePageAndATodoIsAdded() {
        launch()
        openTheQuickAddPageWithOneFinger()

        // The first time there is no project yet: the page opens on the list of them, a group for each Mac.
        _ = requireElement("quick-todo-picker", timeout: 10)
        shot("dev-07-picker")
        requireElement("quick-todo-project-" + askHuman).tap()

        // Back at the text, with the project on top and the keyboard up.
        _ = requireElement("quick-todo-field")
        XCTAssertTrue(element("quick-todo-project").label.contains("AskHuman"), element("quick-todo-project").label)
        XCTAssertFalse(control("quick-todo-send").isEnabled, "there is nothing to add yet")
        typeIntoTheField("Try the quick add")
        XCTAssertTrue(control("quick-todo-send").isEnabled)
        shot("dev-08-page")
        control("quick-todo-send").tap()

        // The page goes and the note says where the todo went (it shows for a moment once the page is gone,
        // so it is waited for first), and then the note goes too.
        let note = requireElement("dev-toast", timeout: 15)
        XCTAssertTrue(note.label.contains("AskHuman"), note.label)
        shot("dev-09-toast")
        XCTAssertFalse(element("quick-todo-field").exists, "the page closes when the Mac has taken the todo")
        XCTAssertTrue(gone("dev-toast", timeout: 8), "the note does not stay")

        // And the todo is in the project.
        element("todo-project-" + askHuman).tap()
        XCTAssertTrue(
            element("todo-project-screen").waitForExistence(timeout: 10)
                && app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "Try the quick add"))
                    .firstMatch.waitForExistence(timeout: 10),
            "the new todo is in the project's queue"
        )
    }

    func testTheProjectIsRememberedAndCancelKeepsTheText() {
        launch()
        openTheQuickAddPageWithOneFinger()
        requireElement("quick-todo-project-" + askHuman).tap()
        typeIntoTheField("Half a thought")
        control("quick-todo-cancel").tap()
        XCTAssertTrue(gone("quick-todo-field", timeout: 10), "Cancel closes the page")
        XCTAssertFalse(element("dev-toast").exists, "a todo that was not added has no note")

        // Next time (by taps this time): straight to the text, with the project known and the text still there.
        button.tap()
        requireElement("dev-menu-quickTodo", timeout: 5).tap()
        let field = requireElement("quick-todo-field", timeout: 10)
        XCTAssertFalse(element("quick-todo-picker").exists, "the project is remembered")
        XCTAssertTrue(element("quick-todo-project").label.contains("AskHuman"))
        XCTAssertEqual(field.value as? String, "Half a thought")
    }

    func testTheProjectCanBeChangedAndFoundBySearching() {
        launch()
        openTheQuickAddPageWithOneFinger()
        _ = requireElement("quick-todo-picker", timeout: 10)
        focusTheSearch()

        // Three projects are called AskHuman, on two Macs: the suffix tells them apart.
        app.typeText("askhuman")
        _ = requireElement("quick-todo-project-" + askHuman)
        _ = requireElement("quick-todo-project-b1b2c3d4e5f60001")
        _ = requireElement("quick-todo-project-b1b2c3d4e5f60002")
        XCTAssertFalse(element("quick-todo-project-a1b2c3d4e5f60002").exists, "Billing Service does not match")
        app.typeText(" playground")
        XCTAssertTrue(element("quick-todo-project-b1b2c3d4e5f60002").waitForExistence(timeout: 5))
        XCTAssertTrue(gone("quick-todo-project-b1b2c3d4e5f60001", timeout: 5), "every word has to match")
        shot("dev-10-search")
        element("quick-todo-project-b1b2c3d4e5f60002").tap()

        _ = requireElement("quick-todo-field")
        let label = element("quick-todo-project").label
        XCTAssertTrue(label.contains("playground") && label.contains("Office iMac"), label)

        // The row on the page takes it back to the list, and another choice replaces the first.
        element("quick-todo-project").tap()
        _ = requireElement("quick-todo-picker", timeout: 10)
        focusTheSearch()
        app.typeText("work")
        requireElement("quick-todo-project-b1b2c3d4e5f60001").tap()
        XCTAssertTrue(element("quick-todo-project").label.contains("work"), element("quick-todo-project").label)
    }

    /// The main window's keyboard goes to the page while it is up, and comes back to what was being typed in.
    func testTheAnswerBeingTypedIsBackInEditingOnceThePageCloses() {
        launch(extra: ["-AHTab", "inbox", "-AHOpen", "req-next"])
        var answer = app.textViews["answer-text-0"]
        for _ in 0..<10 where !(answer.exists && answer.isHittable) {
            app.swipeUp(velocity: .fast)
            answer = app.textViews["answer-text-0"]
        }
        XCTAssertTrue(answer.waitForExistence(timeout: 10))
        answer.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        dismissTheKeyboardTip()
        answer.typeText("My unfinished answer")

        // Up with the page, with a project already chosen (the picker has nothing to do with this).
        button.tap()
        requireElement("dev-menu-quickTodo", timeout: 5).tap()
        _ = requireElement("quick-todo-picker", timeout: 10)
        requireElement("quick-todo-project-" + askHuman).tap()
        _ = requireElement("quick-todo-field", timeout: 10)
        shot("dev-11-over-an-answer")
        control("quick-todo-cancel").tap()
        XCTAssertTrue(gone("quick-todo-field", timeout: 10), "the page closes")

        // The answer is being edited again, the text and the cursor where they were.
        XCTAssertTrue(
            eventually(timeout: 10) { self.app.keyboards.firstMatch.exists },
            "the keyboard comes back for the answer"
        )
        XCTAssertEqual(answer.value as? String, "My unfinished answer")
        answer.typeText(" and the rest")
        XCTAssertEqual(answer.value as? String, "My unfinished answer and the rest")
        shot("dev-12-answer-back")
    }

    /// The same, with the field in a sheet that is up already: the page opens over the sheet, and the sheet's
    /// field is the one being typed in again once the page is gone.
    func testThePageOpensOverASheetAndTheSheetsFieldIsBackInEditingOnceItCloses() {
        launch(extra: ["-AHTab", "agents", "-AHNewTask", "YES"])
        reveal("task-text")
        let field = requireElement("task-text")
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        dismissTheKeyboardTip()
        field.typeText("A task I am writing")

        button.tap()
        requireElement("dev-menu-quickTodo", timeout: 5).tap()
        _ = requireElement("quick-todo-picker", timeout: 10)
        requireElement("quick-todo-project-" + askHuman).tap()
        _ = requireElement("quick-todo-field", timeout: 10)
        shot("dev-13-over-a-sheet")
        control("quick-todo-cancel").tap()
        XCTAssertTrue(gone("quick-todo-field", timeout: 10), "the page closes")

        // The sheet is still up with its text, the keyboard is back, and typing goes on in its field.
        XCTAssertTrue(
            eventually(timeout: 10) { self.app.keyboards.firstMatch.exists },
            "the keyboard comes back for the sheet's field"
        )
        XCTAssertEqual(field.value as? String, "A task I am writing")
        field.typeText(" and more")
        XCTAssertEqual(field.value as? String, "A task I am writing and more")
        shot("dev-14-sheet-field-back")
    }


    // MARK: above everything

    func testTheButtonIsAboveASheetAndItsKeyboardAndRidesAboveTheKeyboard() {
        launch(extra: ["-AHTab", "agents", "-AHNewTask", "YES"])
        reveal("task-text")
        let field = requireElement("task-text")
        // Put the button low, where the keyboard will come: it is above the sheet and takes the finger.
        slide(from: requireElement("dev-button"), hold: 0.01, to: CGPoint(x: screen.width - 30, y: screen.height - 150))
        XCTAssertTrue(eventually { self.button.frame.midY > self.screen.height - 200 }, "\(button.frame)")
        let resting = button.frame

        field.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 10))
        XCTAssertTrue(
            eventually { self.button.frame.maxY <= keyboard.frame.minY + 1 },
            "the button rides above the keyboard: button \(button.frame), keyboard \(keyboard.frame)"
        )
        shot("dev-06-above-keyboard")

        // It still takes a touch there, over the sheet.
        button.tap()
        _ = requireElement("dev-menu-quickTodo", timeout: 5)
        XCTAssertTrue(app.keyboards.firstMatch.exists, "the button's window never takes the keyboard away from the sheet")
        // A tap away from the menu closes it. It lands on the title of the sheet, not on the field: with the
        // keyboard up the field sits level with the button, where the one row of the menu is.
        point(screen.width / 2, 97).tap()
        XCTAssertTrue(gone("dev-menu-quickTodo", timeout: 5))
        XCTAssertFalse(element("quick-todo-picker").exists, "that tap was not on the row")

        // And back where it was left once the keyboard goes.
        control("task-cancel").tap()
        let discard = app.sheets.firstMatch.buttons[localized("Discard", "放弃")]
        if discard.waitForExistence(timeout: 2) { discard.tap() }
        XCTAssertTrue(
            eventually(timeout: 10) { !self.app.keyboards.firstMatch.exists && abs(self.button.frame.midY - resting.midY) < 3 },
            "back at \(resting), now \(button.frame)"
        )
    }
}
