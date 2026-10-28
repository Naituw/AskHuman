import XCTest

/// The Todos tab (`docs/specs/ios-todos.md`) through the real UI over the mock backend, in English or Chinese
/// (`TEST_RUNNER_AH_UI_LANG=zh-Hans`). The demo's Home MacBook Pro syncs its todos through iCloud and the Office
/// iMac keeps its own; "AskHuman" is the project with the most to look at.
@MainActor
final class AskHumanTodosUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    private let askHuman = "a1b2c3d4e5f60001"
    private let officeWork = "b1b2c3d4e5f60001"

    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        // What the screen looked like when a test failed.
        if let run = testRun, run.totalFailureCount > 0, app != nil { shot("failed-" + name.filter { $0.isLetter }) }
    }

    // MARK: helpers

    @discardableResult
    private func launch(scenario: String = "demo", extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", scenario, "-AHLatency", "0.05", "-AHAutoAdvance", "NO", "-AHTab", "todos",
            "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ] + extra
        app.launch()
        self.app = app
        return app
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func requireElement(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let found = element(id)
        XCTAssertTrue(found.waitForExistence(timeout: timeout), "\(id) did not appear", file: file, line: line)
        return found
    }

    private func gone(_ id: String, timeout: TimeInterval = 10) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element(id))
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    /// Anything on screen whose label contains `label` (a row is one button that folds its texts into its label).
    private func text(_ label: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", label)).firstMatch
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

    /// Swipes a row to the right from well inside it: starting at the screen's edge would be the back gesture.
    private func swipeRight(_ id: String) {
        let row = requireElement(id)
        let from = row.coordinate(withNormalizedOffset: CGVector(dx: 0.25, dy: 0.5))
        let to = row.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5))
        from.press(forDuration: 0.05, thenDragTo: to)
    }

    /// The list hands the keyboard back when it is dragged down.
    private func dismissKeyboard() {
        guard app.keyboards.count > 0 else { return }
        let list = element("todo-project-screen")
        list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
            .press(forDuration: 0.05, thenDragTo: list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 1.0)))
    }

    /// Opens the project list's menu and picks a source.
    private func choose(source id: String) {
        requireElement("todo-source-menu").tap()
        requireElement("todo-source-" + id).tap()
    }

    // MARK: tests

    func testTheTabListsTheProjectsOfTheFirstMacWithItsSyncLine() {
        launch()
        _ = requireElement("todo-project-" + askHuman)
        _ = requireElement("todo-sync-line")
        XCTAssertTrue(app.tabBars.buttons[localized("Todos", "待办")].exists)
        shot("todos-projects")
    }

    func testTheSourceMenuSwitchesBetweenICloudAndAMac() {
        launch()
        _ = requireElement("todo-project-" + askHuman)

        requireElement("todo-source-menu").tap()
        _ = requireElement("todo-source-icloud")
        shot("todos-source-menu")
        element("todo-source-icloud").tap()
        _ = requireElement("todo-sync-line")
        XCTAssertTrue(text(localized("via Home MacBook Pro", "经由 Home MacBook Pro")).waitForExistence(timeout: 10), "iCloud names the Mac that serves it")
        shot("todos-icloud")

        choose(source: "mac:mockofficemacmacmacmacmac")
        _ = requireElement("todo-project-" + officeWork)
        XCTAssertFalse(element("todo-sync-line").exists, "the Office iMac does not sync")
        shot("todos-office")
    }

    func testAddingFinishingEditingAndRestoringATodo() {
        launch()
        requireElement("todo-project-" + askHuman).tap()
        _ = requireElement("todo-row-todo-ah-1")

        // Add one.
        let field = requireElement("todo-add-field")
        field.tap()
        field.typeText("Ship the todo tab")
        requireElement("todo-add-send").tap()
        XCTAssertTrue(text("Ship the todo tab").waitForExistence(timeout: 10))
        dismissKeyboard()
        shot("todos-added")

        // Finish the first one: it leaves the queue and shows up in the history.
        swipeRight("todo-row-todo-ah-1")
        app.buttons[localized("Done", "完成")].firstMatch.tap()
        XCTAssertTrue(gone("todo-row-todo-ah-1"), "a finished todo leaves the queue")
        requireElement("todo-history-toggle").tap()
        _ = requireElement("todo-done-todo-ah-1")
        shot("todos-history")

        // Bring it back.
        swipeRight("todo-done-todo-ah-1")
        app.buttons[localized("Restore", "恢复")].firstMatch.tap()
        _ = requireElement("todo-row-todo-ah-1")

        // Edit another one.
        element("todo-row-todo-ah-2").tap()
        let edit = requireElement("todo-edit-field")
        edit.tap()
        edit.typeText(" (edited)")
        shot("todos-edit")
        requireElement("todo-edit-save").tap()
        XCTAssertTrue(text("(edited)").waitForExistence(timeout: 10))
    }

    func testDraggingATodoUpChangesTheOrder() {
        launch()
        requireElement("todo-project-" + askHuman).tap()
        _ = requireElement("todo-row-todo-ah-1")
        _ = requireElement("todo-row-todo-ah-4")
        XCTAssertLessThan(element("todo-row-todo-ah-1").frame.minY, element("todo-row-todo-ah-4").frame.minY)

        // Edit mode shows a handle on every row; the last todo is dragged above the first.
        requireElement("todo-reorder").tap()
        // The handle's label is "Reorder <row>" in English and "对<row>重新排序" in Chinese.
        let handles = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", localized("Reorder", "重新排序")))
        XCTAssertTrue(handles.firstMatch.waitForExistence(timeout: 10), "edit mode offers handles; \(app.debugDescription)")
        let last = handles.element(boundBy: handles.count - 1)
        let start = last.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let first = element("todo-row-todo-ah-1").coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.05))
        // A table only starts to move a row once it has been held and then dragged slowly.
        start.press(forDuration: 1.0, thenDragTo: first, withVelocity: .slow, thenHoldForDuration: 1.0)
        shot("todos-reordered")

        requireElement("todo-reorder").tap()
        let moved = element("todo-row-todo-ah-4").frame.minY < element("todo-row-todo-ah-1").frame.minY
        XCTAssertTrue(moved, "the dragged todo is now above the first one")
    }

    func testDeletingATodoAsksFirst() {
        launch()
        requireElement("todo-project-" + askHuman).tap()
        _ = requireElement("todo-row-todo-ah-3")
        element("todo-row-todo-ah-3").swipeLeft()
        app.buttons[localized("Delete", "删除")].firstMatch.tap()
        // The confirmation names the todo.
        shot("todos-delete-confirm")
        app.buttons[localized("Delete", "删除")].firstMatch.tap()
        XCTAssertTrue(gone("todo-row-todo-ah-3"))
    }

    func testAMacThatPredatesTodosAsksForAnUpdate() {
        launch(scenario: "outdated")
        XCTAssertTrue(
            text(localized("This Mac needs an update", "这台 Mac 需要更新")).waitForExistence(timeout: 20),
            "the page says why there is nothing to show"
        )
        XCTAssertTrue(
            text(localized("Update AskHuman on Home MacBook Pro", "请先更新 Home MacBook Pro 上的 AskHuman")).exists,
            "and names the Mac to update"
        )
        XCTAssertFalse(element("todo-project-" + askHuman).exists, "nothing is asked of a Mac that does not know todos")
        shot("todos-needs-update")

        // The menu lists the Macs, none of which can be chosen.
        requireElement("todo-source-menu").tap()
        let office = requireElement("todo-source-mac:mockofficemacmacmacmacmac")
        XCTAssertFalse(office.isEnabled, "an outdated Mac cannot be chosen")
        XCTAssertTrue(text(localized("Needs an update", "需要更新")).exists, "the menu says what is wrong with it")
        shot("todos-needs-update-menu")
    }

    func testAMacThatCannotBeReachedSaysSo() {
        launch(scenario: "offline")
        _ = requireElement("todo-projects")
        XCTAssertTrue(
            text(localized("Mac not reachable", "Mac 暂时连不上")).waitForExistence(timeout: 20)
                || element("notice-todo-stale").waitForExistence(timeout: 5),
            "an unreachable Mac is named"
        )
        shot("todos-offline")
    }
}
