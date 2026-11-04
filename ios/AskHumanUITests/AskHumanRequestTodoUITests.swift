import CryptoKit
import XCTest

/// The request page's "Add todo…" (`docs/specs/todo-quick-add.md` §4.2) through the real UI over the mock Macs, in
/// English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`). The demo's questions and confirmations all belong to a
/// project of the Home MacBook Pro ("HumanInLoop") or of the Office iMac ("relay").
@MainActor
final class AskHumanRequestTodoUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        // What the screen looked like when a test failed.
        if let run = testRun, run.totalFailureCount > 0, app != nil { shot("failed-" + name.filter { $0.isLetter }) }
    }

    // MARK: helpers

    /// The note that says a todo was added stays 2 seconds in the app: too short for a test that polls a busy
    /// simulator. These are the launch arguments that stretch it.
    private let longNote = ["-AHTodoNoteHold", "12"]

    @discardableResult
    private func launch(request: String = "req-migration", scenario: String = "demo", extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        app.launchArguments = [
            "-AHMock", "YES", "-AHLatency", "0.05", "-AHScenario", scenario, "-AHWebMarkdown", "NO", "-AHOpen", request,
            "-AHAutoAdvance", "NO", "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ] + extra
        app.launch()
        self.app = app
        _ = requireElement("request-menu")
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

    /// The Mac's id of a project: the hash of its key (what `TodoProjectId` makes of it).
    private func projectId(_ key: String) -> String {
        SHA256.hash(data: Data(key.utf8)).prefix(8).map { String(format: "%02x", $0) }.joined()
    }

    /// The menu entry of the page, by its words (a menu item is found by its label).
    private func menuItem(_ en: String, _ zh: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label == %@", localized(en, zh))).firstMatch
    }

    private var addTodoItem: XCUIElement { menuItem("Add todo…", "添加待办…") }

    /// Opens the page's menu and waits for its "Add todo…" item.
    private func openTheMenu(file: StaticString = #filePath, line: UInt = #line) {
        requireElement("request-menu", file: file, line: line).tap()
        XCTAssertTrue(addTodoItem.waitForExistence(timeout: 5), "the menu has no \"Add todo…\"", file: file, line: line)
    }

    /// Chooses "Add todo…" in the open menu and returns the sheet's field.
    ///
    /// The tap goes to the left end of the row, not its middle: the page's pager sits under the first row of the
    /// menu, and a slow tap on the middle of that row (a busy machine delivers a touch late) is sometimes ignored
    /// by the system menu. A tap that does not take leaves the menu up, so it is repeated a few times at most.
    @discardableResult
    private func chooseAddTodo(file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let field = element("request-todo-field")
        for _ in 0..<3 {
            addTodoItem.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5)).tap()
            if field.waitForExistence(timeout: 6) { return field }
            // The menu is gone: the sheet is on its way, there is nothing left to tap.
            if !addTodoItem.exists { break }
        }
        return requireElement("request-todo-field", timeout: 10, file: file, line: line)
    }

    @discardableResult
    private func openTheSheet(file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        openTheMenu(file: file, line: line)
        return chooseAddTodo(file: file, line: line)
    }

    /// What the field holds (its placeholder when it is empty).
    private func typed(_ field: XCUIElement) -> String { (field.value as? String) ?? "" }

    // MARK: the menu and the sheet

    func testTheMenuOffersAddTodoAndTheSheetNamesTheProjectAndTheMac() {
        launch()
        let field = openTheSheet()
        let project = requireElement("request-todo-project")
        XCTAssertTrue(project.label.contains("HumanInLoop"), "the project of the request: \(project.label)")
        XCTAssertTrue(project.label.contains("Home MacBook Pro"), "the Mac of the request: \(project.label)")
        XCTAssertFalse(element("request-todo-note").exists, "nothing stands in the way of an empty text")
        XCTAssertFalse(requireElement("request-todo-send").isEnabled, "an empty text is not added")

        field.typeText("Cover the rollback path")
        XCTAssertTrue(requireElement("request-todo-send").isEnabled)
        shot("160-request-todo-sheet")
    }

    func testTheMenuKeepsItsOtherItems() {
        launch()
        openTheMenu()
        XCTAssertTrue(menuItem("Cancel request", "取消请求").exists, "a question can still be cancelled")
        shot("161-request-menu")
    }

    func testAConfirmationAddsATodoToo() {
        launch(request: "req-stop", extra: longNote)
        openTheMenu()
        XCTAssertTrue(menuItem("Dismiss", "忽略").exists, "a confirmation can still be dismissed")
        let field = chooseAddTodo()
        XCTAssertTrue(requireElement("request-todo-project").label.contains("relay"))
        XCTAssertTrue(requireElement("request-todo-project").label.contains("Office iMac"))
        field.typeText("Pin the relay version")
        requireElement("request-todo-send").tap()
        XCTAssertTrue(requireElement("todo-added-pill", timeout: 10).label.contains("relay"))
    }

    // MARK: adding

    func testAddingATodoClosesTheSheetSaysSoAndLandsOnTheTodosTab() {
        launch(extra: longNote)
        let field = openTheSheet()
        field.typeText("Cover the rollback path")
        requireElement("request-todo-send").tap()

        let pill = requireElement("todo-added-pill", timeout: 10)
        XCTAssertTrue(pill.label.contains("HumanInLoop"), "the note names the project: \(pill.label)")
        XCTAssertTrue(gone("request-todo-field", timeout: 10), "the sheet is closed")
        shot("162-request-todo-added")
        XCTAssertTrue(gone("todo-added-pill", timeout: 40), "the note goes by itself")

        // What was typed went with the todo.
        let again = openTheSheet()
        XCTAssertNotEqual(typed(again), "Cover the rollback path")
        requireElement("request-todo-cancel").tap()
        XCTAssertTrue(gone("request-todo-field"))

        // The Mac has it: the project is on the Todos tab now.
        app.navigationBars.buttons.element(boundBy: 0).tap()
        app.tabBars.buttons.element(boundBy: 2).tap()
        let row = element("todo-project-" + projectId("proj-humaninloop"))
        for _ in 0..<6 where !row.exists { app.swipeUp() }
        XCTAssertTrue(row.waitForExistence(timeout: 10), "the project of the request is listed once it has a todo")
        row.tap()
        _ = requireElement("todo-project-screen", timeout: 10)
        // A row is one element whose label carries its text.
        let todo = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "Cover the rollback path")).firstMatch
        XCTAssertTrue(todo.waitForExistence(timeout: 10), "and so is the todo")
        shot("163-request-todo-on-the-todos-tab")
    }

    func testTheTextIsKeptWhenTheSheetIsCancelled() {
        launch()
        let field = openTheSheet()
        field.typeText("Half written")
        requireElement("request-todo-cancel").tap()
        XCTAssertTrue(gone("request-todo-field"))
        XCTAssertFalse(element("todo-added-pill").exists, "nothing was added")

        let again = openTheSheet()
        XCTAssertEqual(typed(again), "Half written")
    }

    func testTwoTodosInARowEachGetTheirNote() {
        launch(extra: longNote)
        openTheSheet().typeText("First")
        requireElement("request-todo-send").tap()
        _ = requireElement("todo-added-pill", timeout: 10)
        XCTAssertTrue(gone("request-todo-field", timeout: 10))

        // A second todo while the first note is still up: the note stays, and it is the second one's now.
        openTheSheet().typeText("Second")
        requireElement("request-todo-send").tap()
        XCTAssertTrue(gone("request-todo-field", timeout: 10))
        _ = requireElement("todo-added-pill", timeout: 10)
        XCTAssertTrue(gone("todo-added-pill", timeout: 40), "and it goes by itself")
    }

    // MARK: when it cannot be added

    func testAMacThatIsOutOfReachIsSaidAndAddIsDimmed() {
        launch(scenario: "offline")
        let field = openTheSheet()
        field.typeText("An idea")
        let note = requireElement("request-todo-note")
        XCTAssertTrue(note.label.contains("Home MacBook Pro"), "the note names the Mac: \(note.label)")
        XCTAssertTrue(note.label.contains(localized("not reachable", "暂时连不上")), note.label)
        XCTAssertFalse(requireElement("request-todo-send").isEnabled)
        shot("164-request-todo-offline")

        requireElement("request-todo-cancel").tap()
        XCTAssertTrue(gone("request-todo-field"))
        XCTAssertFalse(element("todo-added-pill").exists)
    }

    func testAMacThatNeedsAnUpdateIsSaidAndAddIsDimmed() {
        launch(scenario: "outdated")
        let field = openTheSheet()
        field.typeText("An idea")
        let note = requireElement("request-todo-note")
        XCTAssertTrue(note.label.contains("Home MacBook Pro"), "the note names the Mac: \(note.label)")
        XCTAssertTrue(note.label.contains(localized("Update AskHuman", "更新")), note.label)
        XCTAssertFalse(requireElement("request-todo-send").isEnabled)
        shot("165-request-todo-outdated")
    }
}
