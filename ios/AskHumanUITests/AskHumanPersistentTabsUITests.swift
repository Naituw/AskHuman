import XCTest

/// Detail pages keep the system tabs while their action bars avoid both tabs and the keyboard.
@MainActor
final class AskHumanPersistentTabsUITests: XCTestCase {
    private var app: XCUIApplication!
    private let tabNames = ["Inbox", "Agents", "Todos", "History", "Macs"]

    override func setUp() async throws { continueAfterFailure = false }

    private func launch(_ arguments: [String]) {
        app = XCUIApplication()
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", "demo", "-AHLatency", "0.05", "-AHAutoAdvance", "NO",
            "-AHWebMarkdown", "NO", "-AppleLanguages", "(en)", "-AppleLocale", "en_US",
        ] + arguments
        app.launch()
    }

    @discardableResult
    private func require(_ id: String, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let element = app.descendants(matching: .any).matching(identifier: id).firstMatch
        XCTAssertTrue(element.waitForExistence(timeout: 20), id, file: file, line: line)
        return element
    }

    private func wait(_ condition: @escaping () -> Bool, file: StaticString = #filePath, line: UInt = #line) {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in condition() }, object: app)
        XCTAssertEqual(XCTWaiter().wait(for: [expectation], timeout: 10), .completed, file: file, line: line)
    }

    private func checkTabs(clearOf ids: [String], file: StaticString = #filePath, line: UInt = #line) {
        let controls = ids.map { require($0, file: file, line: line) }
        for name in tabNames {
            let tab = app.tabBars.buttons[name]
            XCTAssertTrue(tab.waitForExistence(timeout: 10), name, file: file, line: line)
            XCTAssertTrue(tab.isHittable, name, file: file, line: line)
            for control in controls {
                XCTAssertFalse(control.frame.intersects(tab.frame), "\(control.identifier) overlaps \(name)", file: file, line: line)
            }
        }
        for control in controls {
            XCTAssertTrue(control.isHittable, control.identifier, file: file, line: line)
            XCTAssertTrue(app.windows.firstMatch.frame.contains(control.frame), "\(control.identifier) is clipped", file: file, line: line)
        }
    }

    private func type(_ text: String, into id: String, send: String) {
        let field = require(id)
        for _ in 0..<8 where !field.isHittable { app.swipeUp(velocity: .slow) }
        field.tap()
        field.typeText(text)
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        let button = require(send)
        wait { button.frame.maxY <= self.app.keyboards.firstMatch.frame.minY + 1 && button.isHittable }
        shot("keyboard-" + id)
    }

    private func dismissKeyboard() {
        for _ in 0..<3 where app.keyboards.firstMatch.exists { app.swipeDown(velocity: .fast) }
        wait { !self.app.keyboards.firstMatch.exists }
    }

    private func reachSend() {
        require("request-title")
        for _ in 0..<8 where !app.buttons["send-button"].exists { app.swipeUp(velocity: .slow) }
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"], !folder.isEmpty {
            let url = URL(fileURLWithPath: folder).appendingPathComponent(name + ".png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
        } else {
            let attachment = XCTAttachment(screenshot: screenshot)
            attachment.name = name
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }

    func testRequestKeepsTabsAndDraftAcrossKeyboardAndTabSwitches() {
        launch(["-AHOpen", "req-next"])
        reachSend()
        checkTabs(clearOf: ["send-button"])
        type("Keep this answer draft.", into: "answer-text-0", send: "send-button")
        dismissKeyboard()
        checkTabs(clearOf: ["send-button"])
        shot("request-tabs")
        app.tabBars.buttons["History"].tap()
        app.tabBars.buttons["Inbox"].tap()
        XCTAssertTrue((require("answer-text-0").value as? String)?.contains("Keep this answer draft.") == true)
        checkTabs(clearOf: ["send-button"])
    }

    /// Where the tab bar and the toolbar share one vertical strip (the outer screen of the Duo, 466 pt wide), the
    /// system's default folds "next request" and the menu into one overflow button to keep all five tabs. The
    /// buttons must stay: the tab bar gives way instead (it becomes the button of the open tab).
    /// (Elsewhere the buttons sit in the navigation bar, and this holds without the preference.)
    func testRequestKeepsEveryToolbarButtonBesideTheTabs() throws {
        launch(["-AHOpen", "req-next"])
        require("request-title")
        try XCTSkipIf(app.windows.firstMatch.frame.width >= 600, "A wide window shows the list beside the request: no back button.")
        for id in ["BackButton", "previous-request", "next-request", "request-menu"] {
            let button = require(id)
            XCTAssertTrue(button.isHittable, id)
            XCTAssertTrue(app.windows.firstMatch.frame.contains(button.frame), "\(id) is clipped")
        }
        XCTAssertFalse(app.descendants(matching: .any).matching(identifier: "BottomOverflowBarButtonItem").firstMatch.exists,
                       "the buttons are not folded into the system's overflow menu")
        shot("request-buttons")
    }

    func testConfirmationKeepsTabsAndSubmitClear() {
        launch(["-AHOpen", "req-permission"])
        checkTabs(clearOf: ["submit-button"])
        shot("confirmation-tabs")
        app.tabBars.buttons["Agents"].tap()
        require("agent-row-claude:cc-flaky")
    }

    func testAgentInputAvoidsTabsAndKeyboard() {
        launch(["-AHTab", "agents", "-AHOpenAgent", "claude:cc-flaky"])
        checkTabs(clearOf: ["interject-field", "interject-send"])
        shot("agent-tabs")
        type("Also run the build.", into: "interject-field", send: "interject-send")
        dismissKeyboard()
        checkTabs(clearOf: ["interject-field", "interject-send"])
        app.tabBars.buttons["Macs"].tap()
        app.tabBars.buttons["Agents"].tap()
        XCTAssertTrue((require("interject-field").value as? String)?.contains("Also run the build.") == true)
    }

    func testTodoInputAvoidsTabsAndKeyboard() {
        launch(["-AHTab", "todos"])
        require("todo-project-a1b2c3d4e5f60001").tap()
        checkTabs(clearOf: ["todo-add-field", "todo-add-send"])
        shot("todo-tabs")
        type("Check the persistent tabs.", into: "todo-add-field", send: "todo-add-send")
        dismissKeyboard()
        checkTabs(clearOf: ["todo-add-field", "todo-add-send"])
        require("todo-add-send").tap()
        let added = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND label CONTAINS %@", "todo-row-", "Check the persistent tabs.")).firstMatch
        XCTAssertTrue(added.waitForExistence(timeout: 10))
        shot("todo-added")
    }

    func testAccessibilityTextKeepsTabsAndActionBarClear() {
        launch(["-AHOpen", "req-next", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityL"])
        reachSend()
        checkTabs(clearOf: ["send-button"])
        shot("accessibility-tabs")
    }
}
