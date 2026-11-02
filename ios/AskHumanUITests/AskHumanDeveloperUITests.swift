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
        element("task-text").coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(gone("dev-menu-quickTodo", timeout: 5))

        // And back where it was left once the keyboard goes.
        element("task-cancel").tap()
        let discard = app.sheets.firstMatch.buttons[localized("Discard", "放弃")]
        if discard.waitForExistence(timeout: 5) { discard.tap() }
        XCTAssertTrue(
            eventually(timeout: 10) { !self.app.keyboards.firstMatch.exists && abs(self.button.frame.midY - resting.midY) < 3 },
            "back at \(resting), now \(button.frame)"
        )
    }
}
