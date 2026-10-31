import XCTest

/// Native window, keyboard and presentation checks over the mock backend; no real Agent is started.
@MainActor
final class AskHumanArrivalUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    override func setUp() async throws { continueAfterFailure = false }

    private func launch(extra: [String] = [], count: Int = 1, slowdown: Double = 20) {
        let app = XCUIApplication()
        app.launchArguments = ["-AHMock", "YES", "-AHAutoAdvance", "NO", "-AHWebMarkdown", "NO", "-AHTab", "agents",
                               "-AHArrival", "Motion verification", "-AHArrivalDelay", "3", "-AHArrivalCount", "\(count)",
                               "-AHArrivalMotionDiagnostics", "YES", "-AHArrivalMotionSlowdown", "\(slowdown)",
                               "-AppleLanguages", "(\(language))", "-AppleLocale", language == "en" ? "en_US" : "zh_CN"] + extra
        app.launch()
        self.app = app
    }

    private func element(_ id: String) -> XCUIElement { app.descendants(matching: .any).matching(identifier: id).firstMatch }

    @discardableResult
    private func require(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let item = element(id)
        XCTAssertTrue(item.waitForExistence(timeout: timeout), "missing \(id)", file: file, line: line)
        return item
    }

    private func status(_ text: String, timeout: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) {
        let view = require("arrival-motion-state", file: file, line: line)
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS %@", text), object: view)
        XCTAssertEqual(XCTWaiter().wait(for: [expectation], timeout: timeout), .completed, "\(view.value ?? "")", file: file, line: line)
    }

    private func reveal(_ id: String) {
        for _ in 0..<8 {
            let field = element(id)
            if field.exists && field.isHittable { return }
            app.swipeUp(velocity: .fast)
        }
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"] {
            let url = URL(fileURLWithPath: folder).appendingPathComponent("\(language)-\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
        } else {
            let attachment = XCTAttachment(screenshot: screenshot)
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }

    func testConcurrentRequestsLandInTheInboxAndTouchesCrossTheBubbleWindow() {
        launch(count: 3)
        status("active=3")
        status("destination=inbox")
        status("key=source")
        XCTAssertFalse(element("banner").exists)
        shot("140-arrival-concurrent")
        let tab = app.tabBars.buttons[language == "en" ? "Todos" : "待办"]
        XCTAssertTrue(tab.exists)
        tab.tap()
        XCTAssertTrue(tab.isSelected, "the decorative window must pass touches to the source tab bar")
        status("touch=pass")
    }

    func testTypingContinuesInTheTaskSheetWhileAnArrivalIsOnScreen() {
        launch(extra: ["-AHNewTask", "YES"], slowdown: 8)
        reveal("task-text")
        let field = require("task-text")
        field.tap()
        field.typeText("Keep typing while the question arrives")
        status("phase=flight", timeout: 25)
        status("destination=title")
        status("key=source")
        XCTAssertTrue(app.keyboards.firstMatch.exists)
        XCTAssertTrue((field.value as? String)?.contains("Keep typing while the question arrives") == true)
        field.typeText(" and preserve this draft")
        XCTAssertTrue((field.value as? String)?.contains("and preserve this draft") == true)
        status("active=1")
        shot("141-arrival-typing")
        require("task-cancel").tap()
        XCTAssertTrue(app.sheets.firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(app.sheets.firstMatch.buttons[language == "en" ? "Discard" : "放弃"].exists)
    }

    func testAFullScreenPageWithoutANavigationTitleStillGetsAPassiveReceiver() {
        launch(extra: ["-AHArrivalFullscreenFixture", "YES"], slowdown: 10)
        _ = require("arrival-fullscreen-fixture")
        status("destination=title")
        status("key=source")
        status("phase=flight", timeout: 30)
        shot("142-arrival-fullscreen")
        require("arrival-fixture-close").tap()
        XCTAssertFalse(element("arrival-fullscreen-fixture").exists)
        status("destination=inbox")
    }

    func testTheNativeMarkedTextAndSelectionSurviveTheTitleReceiverAppearing() {
        launch(extra: ["-AHNewTask", "YES", "-AHArrivalMarkedTextFixture", "YES"], slowdown: 8)
        reveal("task-text")
        let field = require("task-text")
        field.tap()
        status("composition=preserved", timeout: 20)
        status("phase=flight", timeout: 25)
        status("composition=preserved")
        status("key=source")
        shot("144-arrival-marked-text")
        field.typeText("done")
        XCTAssertTrue((field.value as? String)?.contains("done") == true)
        XCTAssertTrue(app.keyboards.firstMatch.exists)
    }

    func testReducedMotionKeepsOnlyTheStaticInboxHint() {
        launch(extra: ["-AHArrivalReducedMotion", "YES"], slowdown: 30)
        status("phase=static")
        status("key=source")
        XCTAssertFalse(element("arrival-bubble-req-live-1").exists)
        XCTAssertFalse(element("banner").exists)
        shot("145-arrival-reduced-motion")
        let tab = app.tabBars.buttons[language == "en" ? "Todos" : "待办"]
        tab.tap()
        XCTAssertTrue(tab.isSelected)
    }

    func testTheImageViewerKeepsItsHeadingAndPagingDuringAnArrival() {
        launch(extra: ["-AHScenario", "gallery", "-AHOpen", "req-gallery"], slowdown: 14)
        let thumbnail = app.buttons["wide-1.png"]
        XCTAssertTrue(thumbnail.waitForExistence(timeout: 15))
        thumbnail.tap()
        let gallery = require("image-gallery")
        status("destination=title")
        status("phase=flight", timeout: 35)
        XCTAssertEqual(require("gallery-name").label, "wide-1.png")
        _ = require("gallery-position")
        shot("146-arrival-gallery")
        gallery.swipeLeft()
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "tall-2.png"), object: require("gallery-name"))
        XCTAssertEqual(XCTWaiter().wait(for: [expectation], timeout: 10), .completed)
        status("key=source")
    }
}
