import XCTest

/// The developer mode (`docs/specs/ios-developer-mode.md`) through the real UI over the mock backend, in
/// English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`): the floating button, its menu, and what a finger can
/// do to them. The button lives in a window of its own, so these also check that it sits above the others and
/// lets the touches that are not its own through. The menu is the system's: what it does with a finger (where
/// it opens, what a slide onto a row does) is not tested here, only that it opens and that its row works.
@MainActor
final class AskHumanDeveloperUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    /// The "AskHuman" project of the demo's iCloud source (the first row of the Todos tab).
    private let askHuman = "a1b2c3d4e5f60001"
    /// The Office iMac does not sync, so it is a source of its own, and "AskHuman · work" and "· playground" are its.
    private let officeSource = "mac:mockofficemacmacmacmacmac"
    private let officeWork = "b1b2c3d4e5f60001"
    private let officePlayground = "b1b2c3d4e5f60002"

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
        isGone(element(id), timeout: timeout)
    }

    private func isGone(_ element: XCUIElement, timeout: TimeInterval = 10) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element)
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

    /// The one row of the button's menu. The menu is the system's, and a system menu's rows carry no
    /// identifier: the row is found by its text (as a button, so that the page's own title is not it).
    private var menuRow: XCUIElement {
        app.buttons.matching(NSPredicate(format: "label == %@", localized("Quick todo", "快速待办"))).firstMatch
    }

    /// Opens the page the way a tap does: the button, then its one row.
    private func openTheQuickAddPage() {
        requireElement("dev-button").tap()
        XCTAssertTrue(menuRow.waitForExistence(timeout: 5), "the menu did not open")
        menuRow.tap()
    }

    /// The choice of the project as a user makes it: the source first (the page opens on the sources while no
    /// project is chosen), then the project.
    private func chooseProject(_ id: String, from source: String = "icloud") {
        _ = requireElement("quick-todo-picker", timeout: 10)
        requireElement("quick-todo-source-" + source).tap()
        requireElement("quick-todo-project-" + id).tap()
    }

    /// The list of projects is lazy and the sheet is half the screen high: swiping up lifts the sheet and then
    /// scrolls down the list (`down: false`: back up it), until the row is there.
    private func scrollTheProjects(to id: String, down: Bool = true) {
        let list = element("quick-todo-projects")
        for _ in 0..<4 where !element("quick-todo-project-" + id).exists {
            if down { list.swipeUp(velocity: .slow) } else { list.swipeDown(velocity: .slow) }
        }
    }

    private func typeIntoTheField(_ text: String) {
        let field = requireElement("quick-todo-field")
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10), "the page opens with the keyboard up")
        dismissTheKeyboardTip()
        field.typeText(text)
    }

    private var origin: XCUICoordinate { app.coordinate(withNormalizedOffset: .zero) }

    private func point(_ x: CGFloat, _ y: CGFloat) -> XCUICoordinate {
        origin.withOffset(CGVector(dx: x, dy: y))
    }

    private var screen: CGRect { app.windows.firstMatch.frame }

    /// A spot that no menu covers (the menu opens near the button, at the right or the left edge) and that has
    /// nothing a tap could do: a tap there closes an open menu and goes no further.
    private var spotAwayFromTheMenu: XCUICoordinate { point(screen.width / 2, 97) }

    /// A finger goes down on the middle of `from`, stays `hold` seconds, moves to `target` (in points of the
    /// screen) and lets go.
    private func slide(from: XCUIElement, hold: TimeInterval, to target: CGPoint) {
        from.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: hold, thenDragTo: point(target.x, target.y))
    }

    /// Opens the menu with a tap, waits for its row and closes it again; that row's place is where the menu
    /// opens from this button (a finger that is down cannot look: where to slide to is learned first).
    private func learnWhereTheRowOpens() -> CGPoint {
        requireElement("dev-button").tap()
        XCTAssertTrue(menuRow.waitForExistence(timeout: 5), "the menu did not open")
        let frame = menuRow.frame
        spotAwayFromTheMenu.tap()
        XCTAssertTrue(isGone(menuRow, timeout: 5), "a tap away from the menu closes it")
        return CGPoint(x: frame.midX, y: frame.midY)
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
        // The project under the tap that closes the menu: its place is read while nothing covers it.
        let project = requireElement("todo-project-" + askHuman)
        let projectSpot = point(project.frame.minX + 40, project.frame.midY)

        button.tap()
        XCTAssertTrue(menuRow.waitForExistence(timeout: 5), "a tap opens the menu")
        shot("dev-03-menu")
        RunLoop.current.run(until: Date().addingTimeInterval(0.6))
        XCTAssertTrue(menuRow.exists, "a tap leaves the menu open")

        // A tap outside closes it, and does not go on to the project under it.
        projectSpot.tap()
        XCTAssertTrue(isGone(menuRow, timeout: 5), "a tap outside closes the menu")
        XCTAssertFalse(element("todo-project-screen").exists, "the tap that closed the menu was not the screen's")

        // The next one is the screen's.
        project.tap()
        _ = requireElement("todo-project-screen", timeout: 10)
        XCTAssertTrue(button.exists, "the button is above the project page too")
    }

    func testTouchesThatAreNotTheButtonsReachTheAppUnderIt() {
        launch()
        tab(4)
        XCTAssertTrue(app.tabBars.buttons.element(boundBy: 4).isSelected, "the tab bar takes touches with the button up")
        XCTAssertTrue(button.exists)
    }

    func testLettingGoOutsideTheMenuChoosesNothingAndDoesNotMoveTheButton() {
        launch()
        let before = button.frame
        slide(from: button, hold: 0.8, to: CGPoint(x: 60, y: 120))
        RunLoop.current.run(until: Date().addingTimeInterval(0.8))
        // What the system does with such a finger (it leaves the menu open, today) is its business; the
        // menu is closed here so that the button can be looked at.
        if menuRow.exists { spotAwayFromTheMenu.tap() }
        XCTAssertTrue(isGone(menuRow, timeout: 5), "the menu can be closed")
        XCTAssertFalse(element("quick-todo-picker").exists, "nothing was chosen")
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
        XCTAssertFalse(menuRow.exists, "dragging does not open the menu")
        shot("dev-04-dragged-left")

        // It keeps its place when the app moves on.
        tab(2)
        XCTAssertLessThan(button.frame.midX, 60)
        XCTAssertEqual(button.frame.midY, 600, accuracy: 24)

        // From the left edge the menu opens too, and all of it is on the screen.
        button.tap()
        XCTAssertTrue(menuRow.waitForExistence(timeout: 5), "the menu opens from the left edge")
        XCTAssertTrue(screen.contains(menuRow.frame), "the menu is on the screen: \(menuRow.frame)")
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

    /// The same finger as a user's: down on the button, a moment's rest while the system opens the menu under
    /// it, then to the row, and up. The finger does not let go before it is on the row.
    private func openTheQuickAddPageWithOneFinger() {
        let row = learnWhereTheRowOpens()
        button.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 1.0, thenDragTo: point(row.x, row.y), withVelocity: .slow, thenHoldForDuration: 0.4)
    }

    func testPressSlideAndLetGoOnTheRowOpensThePageAndATodoIsAdded() {
        launch()
        openTheQuickAddPageWithOneFinger()

        // The first time there is no project yet: the page opens on the sources, as the Todos tab has them.
        // The Home MacBook Pro syncs, so it is what iCloud is made of; the Office iMac is a source of its own.
        _ = requireElement("quick-todo-picker", timeout: 10)
        requireElement("quick-todo-source-icloud")
        requireElement("quick-todo-source-" + officeSource)
        XCTAssertFalse(element("quick-todo-source-mac:mockhomemacmacmacmacmacmac").exists, "a Mac that syncs is part of iCloud")
        shot("dev-07-sources")

        // iCloud's projects, grouped as on the Todos tab; what is only on the Office iMac is not among them.
        requireElement("quick-todo-source-icloud").tap()
        _ = requireElement("quick-todo-projects", timeout: 10)
        requireElement("quick-todo-project-" + askHuman)
        XCTAssertFalse(element("quick-todo-project-" + officeWork).exists, "the Office iMac's projects are not iCloud's")
        shot("dev-07-projects")
        element("quick-todo-project-" + askHuman).tap()

        // Back at the text, with the project on top (it says iCloud) and the keyboard up.
        _ = requireElement("quick-todo-field")
        let chosen = element("quick-todo-project").label
        XCTAssertTrue(chosen.contains("AskHuman") && chosen.contains("iCloud"), chosen)
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
        openTheQuickAddPage()
        chooseProject(askHuman)
        typeIntoTheField("Half a thought")
        control("quick-todo-cancel").tap()
        XCTAssertTrue(gone("quick-todo-field", timeout: 10), "Cancel closes the page")
        XCTAssertFalse(element("dev-toast").exists, "a todo that was not added has no note")

        // Next time: straight to the text, with the project known and the text still there.
        openTheQuickAddPage()
        let field = requireElement("quick-todo-field", timeout: 10)
        XCTAssertFalse(element("quick-todo-picker").exists, "the project is remembered")
        XCTAssertTrue(element("quick-todo-project").label.contains("AskHuman"))
        XCTAssertEqual(field.value as? String, "Half a thought")
    }

    func testTheProjectCanBeChangedAndFoundBySearching() {
        launch()
        openTheQuickAddPage()
        // The Office iMac is a source of its own: its projects are the ones searched.
        _ = requireElement("quick-todo-picker", timeout: 10)
        requireElement("quick-todo-source-" + officeSource).tap()
        _ = requireElement("quick-todo-projects", timeout: 10)
        focusTheSearch()

        // Two of its projects are called AskHuman: the suffix tells them apart.
        app.typeText("askhuman")
        _ = requireElement("quick-todo-project-" + officeWork)
        _ = requireElement("quick-todo-project-" + officePlayground)
        XCTAssertFalse(element("quick-todo-project-b1b2c3d4e5f60003").exists, "sandbox does not match")
        app.typeText(" playground")
        XCTAssertTrue(element("quick-todo-project-" + officePlayground).waitForExistence(timeout: 5))
        XCTAssertTrue(gone("quick-todo-project-" + officeWork, timeout: 5), "every word has to match")
        shot("dev-10-search")
        element("quick-todo-project-" + officePlayground).tap()

        _ = requireElement("quick-todo-field")
        let label = element("quick-todo-project").label
        XCTAssertTrue(label.contains("playground") && label.contains("Office iMac"), label)

        // The row on the page takes it back to the projects of that source, and another choice replaces the first.
        element("quick-todo-project").tap()
        _ = requireElement("quick-todo-projects", timeout: 10)
        focusTheSearch()
        app.typeText("work")
        requireElement("quick-todo-project-" + officeWork).tap()
        XCTAssertTrue(element("quick-todo-project").label.contains("work"), element("quick-todo-project").label)

        // The back button of that list is the way to the sources: another source, another project. (The page
        // behind the sheet has a "Source" button too, so the list's own bar is the one looked in.)
        element("quick-todo-project").tap()
        _ = requireElement("quick-todo-projects", timeout: 10)
        app.navigationBars["Office iMac"].buttons[localized("Source", "来源")].tap()
        requireElement("quick-todo-source-icloud").tap()
        requireElement("quick-todo-project-" + askHuman).tap()
        let icloud = element("quick-todo-project").label
        XCTAssertTrue(icloud.contains("AskHuman") && icloud.contains("iCloud") && !icloud.contains("Office"), icloud)
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
        openTheQuickAddPage()
        chooseProject(askHuman)
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

        openTheQuickAddPage()
        chooseProject(askHuman)
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

        // It still takes a touch there, over the sheet, and the menu comes up over the sheet too.
        button.tap()
        XCTAssertTrue(menuRow.waitForExistence(timeout: 5), "the menu opens over the sheet")
        XCTAssertTrue(app.keyboards.firstMatch.exists, "the button's menu never takes the keyboard away from the sheet")
        // A tap away from the menu closes it. It lands on the title of the sheet, not on the field.
        spotAwayFromTheMenu.tap()
        XCTAssertTrue(isGone(menuRow, timeout: 5))
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

    func testTheButtonIsAboveAFullScreenImageViewerAndThePageOpensOverItToo() {
        launch(scenario: "gallery", extra: ["-AHWebMarkdown", "NO", "-AHOpen", "req-gallery"])
        let thumbnail = app.buttons["wide-1.png"]
        XCTAssertTrue(thumbnail.waitForExistence(timeout: 15))
        thumbnail.tap()
        let gallery = requireElement("image-gallery")
        XCTAssertTrue(button.waitForExistence(timeout: 5), "the viewer covers the screen, and the button is on it")
        XCTAssertTrue(button.isHittable, "nothing of the viewer is over the button")
        shot("dev-15-over-the-viewer")

        // Everywhere else the screen is still the viewer's: a swipe pages.
        gallery.swipeLeft()
        XCTAssertTrue(
            eventually(timeout: 10) { self.element("gallery-name").label == "tall-2.png" },
            "the viewer pages under the button: \(element("gallery-name").label)"
        )

        // The page opens over the viewer, and the viewer is as it was once the page is gone.
        openTheQuickAddPage()
        chooseProject(askHuman)
        _ = requireElement("quick-todo-field", timeout: 10)
        shot("dev-16-page-over-the-viewer")
        control("quick-todo-cancel").tap()
        XCTAssertTrue(gone("quick-todo-field", timeout: 10), "the page closes")
        XCTAssertEqual(element("gallery-name").label, "tall-2.png", "the viewer is where it was")
        XCTAssertTrue(button.isHittable)
    }

    // MARK: nothing to add to

    /// No Mac can take a todo: every source is listed dimmed with why not, there is nothing to open, and the page says so.
    private func expectNoMacCanTakeATodo(scenario: String, reason: String) {
        launch(scenario: scenario)
        openTheQuickAddPage()
        _ = requireElement("quick-todo-picker", timeout: 10)

        XCTAssertTrue(app.staticTexts[reason].firstMatch.waitForExistence(timeout: 10), "no reason is given")
        XCTAssertEqual(app.staticTexts.matching(NSPredicate(format: "label == %@", reason)).count, 2, "one for each Mac")
        XCTAssertTrue(
            app.staticTexts[localized("No Mac has iCloud sync on", "没有 Mac 开启了 iCloud 同步")].exists,
            "iCloud says that nothing in it can be reached"
        )
        let sources = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "quick-todo-source-"))
        XCTAssertEqual(sources.count, 3, "iCloud and the two Macs")
        for index in 0..<sources.count {
            XCTAssertFalse(sources.element(boundBy: index).isEnabled, "\(sources.element(boundBy: index).identifier) can be opened")
        }
        XCTAssertFalse(
            app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "quick-todo-project-")).firstMatch.exists,
            "there is nothing to choose"
        )
        shot("dev-17-\(scenario)")

        // Back on the page nothing is chosen, and that is all it says: the text is empty, it needs no explaining.
        app.navigationBars.buttons[localized("Quick todo", "快速待办")].tap()
        let note = requireElement("quick-todo-note", timeout: 5)
        XCTAssertEqual(note.label, localized("Choose a project first.", "请先选择项目。"))
        XCTAssertFalse(control("quick-todo-send").isEnabled, "nothing can be added")
        control("quick-todo-cancel").tap()
        XCTAssertTrue(gone("quick-todo-field", timeout: 10), "the page can still be closed")
    }

    func testWhenNoMacIsReachableTheListSaysWhyAndNothingCanBeAdded() {
        expectNoMacCanTakeATodo(scenario: "offline", reason: localized("Offline", "离线"))
    }

    func testAMacThatPredatesTodosIsListedWithTheUpdateItNeeds() {
        expectNoMacCanTakeATodo(scenario: "outdated", reason: localized("Needs an update", "需要更新"))
    }

    // MARK: two Macs that sync

    /// Two Macs that both sync through iCloud are one source, so there is nothing to choose between: the page
    /// opens on its projects. A project both have is listed once; what only one has is listed under its Mac.
    func testMacsThatBothSyncAreOneICloudWhoseProjectsAreListedOnce() {
        launch(scenario: "bothSync")
        openTheQuickAddPage()
        _ = requireElement("quick-todo-projects", timeout: 15)
        XCTAssertFalse(element("quick-todo-picker").exists, "one source is no choice")

        // "AskHuman" is on both Macs, and the Todos tab lists it once; each Mac's own projects are in a group of its
        // own, further down (the sheet is half the screen high: swiping up lifts it and then scrolls).
        _ = requireElement("quick-todo-project-" + askHuman)
        XCTAssertFalse(element("quick-todo-project-" + officeWork).exists, "the Office iMac's AskHuman is the same project")
        scrollTheProjects(to: "b1b2c3d4e5f60004")
        _ = requireElement("quick-todo-project-a1b2c3d4e5f60006")
        XCTAssertTrue(app.staticTexts[localized("Only on Home MacBook Pro", "仅在 Home MacBook Pro")].exists)
        XCTAssertTrue(app.staticTexts[localized("Only on Office iMac", "仅在 Office iMac")].exists)
        shot("dev-20-both-sync")

        // A project of iCloud is iCloud's, not one Mac's: the page says so.
        scrollTheProjects(to: askHuman, down: false)
        requireElement("quick-todo-project-" + askHuman).tap()
        _ = requireElement("quick-todo-field", timeout: 10)
        let label = element("quick-todo-project").label
        XCTAssertTrue(label.contains("AskHuman") && label.contains("iCloud") && !label.contains("MacBook"), label)

        // A project that only the Office iMac has stays that Mac's.
        element("quick-todo-project").tap()
        _ = requireElement("quick-todo-projects", timeout: 10)
        scrollTheProjects(to: "b1b2c3d4e5f60004")
        requireElement("quick-todo-project-b1b2c3d4e5f60004").tap()
        let intranet = element("quick-todo-project").label
        XCTAssertTrue(intranet.contains("intranet") && intranet.contains("Office iMac"), intranet)
    }

    // MARK: while a request arrives

    /// A request that arrives `seconds` after the launch and flies for a long while (the app's own test hook).
    private func arrival(after seconds: Int, slowdown: Int) -> [String] {
        ["-AHArrival", "Motion verification", "-AHArrivalDelay", "\(seconds)", "-AHArrivalCount", "1",
         "-AHArrivalMotionDiagnostics", "YES", "-AHArrivalMotionSlowdown", "\(slowdown)", "-AHWebMarkdown", "NO", "-AHTab", "agents"]
    }

    private func arrivalStatus(_ text: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) {
        let view = requireElement("arrival-motion-state", file: file, line: line)
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS %@", text), object: view)
        XCTAssertEqual(XCTWaiter().wait(for: [expectation], timeout: timeout), .completed, "\(view.value ?? "")", file: file, line: line)
    }

    func testTheButtonTakesItsOwnTouchesAndPassesTheRestWhileARequestArrives() {
        launch(extra: arrival(after: 3, slowdown: 20))
        _ = requireElement("dev-button")
        arrivalStatus("active=1", timeout: 30)
        XCTAssertTrue(button.isHittable, "the button is above the window the bubble flies in")
        shot("dev-18-while-a-request-arrives")

        button.tap()
        XCTAssertTrue(menuRow.waitForExistence(timeout: 5), "the menu opens during the flight")
        spotAwayFromTheMenu.tap()
        XCTAssertTrue(isGone(menuRow, timeout: 5), "and closes during it")

        // A touch anywhere else goes through both windows to the tab bar under them.
        let todos = app.tabBars.buttons[localized("Todos", "待办")]
        todos.tap()
        XCTAssertTrue(todos.isSelected, "the windows above the app pass the touch on")
        arrivalStatus("active=1")
    }

    func testTypingContinuesInThePageWhileARequestArrivesUnderIt() {
        launch(extra: arrival(after: 18, slowdown: 12))
        openTheQuickAddPage()
        chooseProject(askHuman)
        let field = requireElement("quick-todo-field", timeout: 10)
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        dismissTheKeyboardTip()
        field.typeText("Typing while it arrives")

        arrivalStatus("active=1", timeout: 40)
        XCTAssertTrue(app.keyboards.firstMatch.exists, "the arrival does not take the keyboard from the page")
        XCTAssertEqual(field.value as? String, "Typing while it arrives")
        field.typeText(" and goes on")
        XCTAssertEqual(field.value as? String, "Typing while it arrives and goes on")
        arrivalStatus("active=1")
        shot("dev-19-page-while-a-request-arrives")

        control("quick-todo-cancel").tap()
        XCTAssertTrue(gone("quick-todo-field", timeout: 10), "the page closes")
    }
}
