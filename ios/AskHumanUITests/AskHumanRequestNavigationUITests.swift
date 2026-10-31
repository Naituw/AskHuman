import XCTest

@MainActor
final class AskHumanRequestNavigationUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    override func setUp() async throws { continueAfterFailure = false }

    private func launch(request: String = "req-migration", scenario: String = "demo", extra: [String] = []) {
        let app = XCUIApplication()
        app.launchArguments = ["-AHMock", "YES", "-AHLatency", "0.05", "-AHScenario", scenario,
                               "-AHWebMarkdown", "NO", "-AHOpen", request, "-AHAutoAdvance", "NO",
                               "-AppleLanguages", "(\(language))", "-AppleLocale", language == "en" ? "en_US" : "zh_CN"] + extra
        if let size = ProcessInfo.processInfo.environment["AH_UI_TEXT_SIZE"] { app.launchArguments += ["-UIPreferredContentSizeCategoryName", size] }
        app.launch()
        self.app = app
        _ = require("request-menu")
    }

    private func element(_ id: String) -> XCUIElement { app.descendants(matching: .any).matching(identifier: id).firstMatch }
    @discardableResult
    private func require(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let item = element(id)
        XCTAssertTrue(item.waitForExistence(timeout: timeout), "missing \(id)", file: file, line: line)
        return item
    }

    private func reveal(_ id: String) -> XCUIElement {
        for _ in 0..<10 {
            let item = element(id)
            if item.exists && item.isHittable && item.frame.midY < app.frame.maxY - 130 { return item }
            app.swipeUp(velocity: .fast)
        }
        return require(id)
    }

    private func title(_ expected: String, file: StaticString = #filePath, line: UInt = #line) {
        let label = require("request-title", file: file, line: line)
        let wait = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", expected), object: label)
        XCTAssertEqual(XCTWaiter().wait(for: [wait], timeout: 12), .completed, file: file, line: line)
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"] {
            let url = URL(fileURLWithPath: folder).appendingPathComponent("\(language)-\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
        } else {
            let attachment = XCTAttachment(screenshot: screenshot); attachment.lifetime = .keepAlways; add(attachment)
        }
    }

    func testTheRightHandArrowsSwitchOnceAndMoreStaysAtTheEdge() {
        launch()
        let previous = require("previous-request"), next = require("next-request"), menu = require("request-menu")
        XCTAssertLessThan(previous.frame.midX, next.frame.midX)
        XCTAssertLessThan(next.frame.midX, menu.frame.midX)
        title("Plan the users table migration")
        shot("150-navigation-toolbar")
        next.tap()
        title("收尾 iOS 通知与分组的细节")
        require("previous-request").tap()
        title("Plan the users table migration")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(element("inbox-row-req-migration").waitForExistence(timeout: 10), "back returns directly to the list")
        XCTAssertFalse(element("next-request").exists)
    }

    func testSwitchingHidesTheKeyboardAndPreservesEachDraft() {
        launch()
        let original = reveal("answer-text-2")
        original.tap(); original.typeText("Keep the migration draft")
        XCTAssertTrue(app.keyboards.firstMatch.exists)
        require("next-request").tap()
        title("收尾 iOS 通知与分组的细节")
        XCTAssertFalse(app.keyboards.firstMatch.exists)
        let other = reveal("answer-text-0")
        other.tap(); other.typeText("A different next-task draft")
        require("previous-request").tap()
        XCTAssertFalse(app.keyboards.firstMatch.exists)
        let restored = reveal("answer-text-2")
        XCTAssertEqual(restored.value as? String, "Keep the migration draft")
        shot("151-navigation-draft-restored")
        require("next-request").tap()
        XCTAssertEqual(reveal("answer-text-0").value as? String, "A different next-task draft")
    }

    func testManualReturnRestoresTheReadingPositionOfALongForm() {
        launch(scenario: "longs", extra: ["-AHWebMarkdown", "YES"])
        let anchor = reveal("answer-text-2")
        let y = anchor.frame.minY
        require("next-request").tap()
        title("Plan the orders table migration")
        require("previous-request").tap()
        let restored = require("answer-text-2")
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            restored.isHittable && abs(restored.frame.minY - y) < 45
        }, object: nil)
        XCTAssertEqual(XCTWaiter().wait(for: [expectation], timeout: 15), .completed)
        shot("152-navigation-reading-restored")
    }

    func testConfirmationChoiceAndInputSurviveAManualRoundTrip() {
        launch(request: "req-stop")
        require("choice-1").tap()
        let input = reveal("confirm-input")
        input.tap(); input.typeText("Keep checking the Windows build")
        require("previous-request").tap()
        XCTAssertFalse(app.keyboards.firstMatch.exists)
        require("next-request").tap()
        XCTAssertEqual(reveal("confirm-input").value as? String, "Keep checking the Windows build")
        XCTAssertFalse(require("next-request").isEnabled)
        shot("153-navigation-confirmation")
    }

    func testASingleRequestHasNoExtraNavigationControls() {
        launch(request: "req-select", scenario: "single")
        XCTAssertFalse(element("previous-request").exists)
        XCTAssertFalse(element("next-request").exists)
        XCTAssertTrue(require("request-menu").exists)
        shot("154-navigation-single")
    }

    func testTheSharedMoreMenuCanStillDismissAConfirmation() {
        launch(request: "req-stop")
        require("request-menu").tap()
        let label = language == "en" ? "Dismiss" : "忽略"
        let dismiss = app.buttons.matching(NSPredicate(format: "label == %@", label)).firstMatch
        XCTAssertTrue(dismiss.waitForExistence(timeout: 5))
        dismiss.tap()
        let confirm = app.buttons.matching(NSPredicate(format: "label == %@", label)).firstMatch
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        confirm.tap()
        XCTAssertTrue(element("inbox-row-req-permission").waitForExistence(timeout: 15))
        XCTAssertFalse(element("inbox-row-req-stop").exists)
    }

    func testArrivalHintLeavesTheRightHandControlsUsableDuringFlight() {
        launch(extra: ["-AHArrival", "Arriving while browsing", "-AHArrivalDelay", "2", "-AHArrivalCount", "3",
                       "-AHArrivalMotionDiagnostics", "YES", "-AHArrivalMotionSlowdown", "8"])
        let diagnostic = require("arrival-motion-state")
        let flight = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS %@", "phase=flight"), object: diagnostic)
        XCTAssertEqual(XCTWaiter().wait(for: [flight], timeout: 25), .completed)
        shot("157-navigation-arrival-receiver")
        require("next-request").tap()
        title("收尾 iOS 通知与分组的细节")
        shot("158-navigation-during-flight")
    }

    func testSwitchingCommitsNativeMarkedTextWhileArrivalMotionRemainsPassive() {
        launch(extra: ["-AHArrival", "Arriving during navigation", "-AHArrivalDelay", "8", "-AHArrivalCount", "3",
                       "-AHArrivalMotionDiagnostics", "YES", "-AHArrivalMotionSlowdown", "30", "-AHArrivalMarkedTextFixture", "YES"])
        let input = reveal("answer-text-2")
        input.tap()
        let diagnostic = require("arrival-motion-state", timeout: 25)
        let marked = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS %@", "composition=preserved"), object: diagnostic)
        XCTAssertEqual(XCTWaiter().wait(for: [marked], timeout: 20), .completed)
        shot("155-navigation-marked-before")
        require("next-request").tap()
        title("收尾 iOS 通知与分组的细节")
        XCTAssertFalse(app.keyboards.firstMatch.exists)
        require("previous-request").tap()
        XCTAssertTrue((reveal("answer-text-2").value as? String)?.contains("ceshi") == true)
        shot("156-navigation-marked-committed")
    }
}
