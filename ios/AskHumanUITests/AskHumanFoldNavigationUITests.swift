import XCTest
import Darwin

/// Run through ios/Tools/test-fold-navigation.py, which folds the Duo when a test asks it to.
/// Starting with an already-open request does not reproduce the lost toolbar: pick it from the list.
@MainActor
final class AskHumanFoldNavigationUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() async throws {
        continueAfterFailure = false
        try XCTSkipUnless(ProcessInfo.processInfo.environment["AH_DUO_FOLD_TESTS"] == "YES",
                          "Requires the Duo host driver to open and close the device.")
        app = XCUIApplication()
    }

    private func launch(tab: String = "inbox") {
        // Start on the inner display before establishing the app's automation session. This is the
        // reported path, and keeps a previous test's folded display out of the next test's setup.
        app.terminate()
        resize(open: true, waitForApp: false)
        app.launchArguments = ["-AHMock", "YES", "-AHLatency", "0.05", "-AHWebMarkdown", "NO",
                               "-AHDeveloperMode", "NO", "-AHAutoAdvance", "NO", "-AHTab", tab,
                               "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
        XCTAssertGreaterThanOrEqual(app.windows.firstMatch.frame.width, 700)
    }

    private struct FoldState: Decodable {
        let nonce: String
        let activeScreen: String
    }

    private func resize(open: Bool, waitForApp: Bool = true) {
        let nonce = UUID().uuidString
        let acknowledgement = URL.documentsDirectory.appendingPathComponent("duo-fold-state.json")
        print("AH_DUO_FOLD_COMMAND \(open ? "open" : "close") \(nonce)")
        fflush(stdout)
        let resized = XCTNSPredicateExpectation(predicate: NSPredicate { [self] _, _ in
            guard let data = try? Data(contentsOf: acknowledgement),
                  let state = try? JSONDecoder().decode(FoldState.self, from: data),
                  state.nonce == nonce, state.activeScreen == (open ? "inner" : "cover") else { return false }
            if !waitForApp { return true }
            let width = app.windows.firstMatch.frame.width
            return open ? width >= 700 : width < 600
        }, object: nil)
        XCTAssertEqual(XCTWaiter().wait(for: [resized], timeout: 60), .completed)
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    @discardableResult
    private func require(_ id: String) -> XCUIElement {
        let item = element(id)
        let visible = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true AND isHittable == true"), object: item)
        let result = XCTWaiter().wait(for: [visible], timeout: 25)
        if result != .completed {
            screenshot("missing-\(id)")
            print(app.debugDescription)
        }
        XCTAssertEqual(result, .completed, "missing or hidden \(id)")
        return item
    }

    private func screenshot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testFoldingAPickedRequestKeepsTheToolbarAndDraft() {
        launch()
        require("inbox-row-req-migration").tap()
        _ = require("request-title")
        // The short next request leaves room for its answer field in both layouts.
        require("next-request").tap()
        _ = require("request-title")
        let answer = require("answer-text-0")
        answer.tap()
        answer.typeText("Keep this draft across folding")
        for cycle in 0..<3 {
            resize(open: false)
            _ = require("BackButton")
            if app.keyboards.firstMatch.exists {
                // The system legitimately overflows actions when the keyboard takes bar space.
                // Dismiss it with the request's interactive scroll gesture before checking all items.
                let body = app.scrollViews.containing(.textView, identifier: "answer-text-0").firstMatch
                let top = app.keyboards.firstMatch.frame.minY - body.frame.minY
                let origin = body.coordinate(withNormalizedOffset: .zero)
                origin.withOffset(CGVector(dx: body.frame.width * 0.4, dy: top * 0.2))
                    .press(forDuration: 0.05, thenDragTo: origin.withOffset(CGVector(dx: body.frame.width * 0.4, dy: app.windows.firstMatch.frame.maxY - body.frame.minY - 10)))
                let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.keyboards.firstMatch)
                XCTAssertEqual(XCTWaiter().wait(for: [dismissed], timeout: 15), .completed)
            }
            _ = require("request-menu")
            _ = require("previous-request")
            _ = require("next-request")
            XCTAssertEqual(element("answer-text-0").value as? String, "Keep this draft across folding")
            screenshot("folded-request-\(cycle)")
            if cycle < 2 { resize(open: true) }
        }
        XCUIDevice.shared.press(.home)
        app.activate()
        for button in ["BackButton", "request-menu", "previous-request", "next-request"] { _ = require(button) }
        XCTAssertEqual(element("answer-text-0").value as? String, "Keep this draft across folding")
        screenshot("folded-request-after-foreground")
        require("previous-request").tap()
        XCTAssertEqual(require("request-title").label, "Plan the users table migration")
        require("BackButton").tap()
        _ = require("inbox-row-req-migration")
        XCTAssertFalse(element("request-title").exists)
    }

    func testFoldingAPickedAgentKeepsTheBackButton() {
        checkFoldedSelection(tab: "agents", row: "agent-row-claude:cc-flaky", detail: "watch-card")
    }

    private func checkFoldedSelection(tab: String, row: String, detail: String) {
        launch(tab: tab)
        require(row).tap()
        _ = require(detail)
        for cycle in 0..<3 {
            resize(open: false)
            _ = require("BackButton")
            _ = require(detail)
            screenshot("folded-\(tab)-\(cycle)")
            if cycle < 2 { resize(open: true) }
        }
        require("BackButton").tap()
        _ = require(row)
        XCTAssertFalse(element(detail).exists)
    }

    func testFoldingAPickedProjectKeepsTheSelectionAndReturn() {
        checkFoldedSelection(tab: "todos", row: "todo-project-a1b2c3d4e5f60001", detail: "todo-project-screen")
    }

    func testFoldingAPickedHistoryEntryKeepsTheSelectionAndReturn() {
        checkFoldedSelection(tab: "history", row: "history-row-req-old-1", detail: "history-title")
    }

    func testFoldingAPickedMacKeepsTheSelectionAndReturn() {
        checkFoldedSelection(tab: "macs", row: "mac-row-mockhomemacmacmacmacmacmac", detail: "mac-detail")
    }

    func testFoldingANewTaskKeepsTheSheetAndDraft() {
        launch(tab: "agents")
        require("new-task").tap()
        _ = require("task-cancel")
        let text = "Keep this task draft across folding"
        let field = element("task-text")
        let bar = app.navigationBars["New task"].frame
        let form = app.collectionViews.allElementsBoundByIndex.first {
            abs($0.frame.minX - bar.minX) < 2 && abs($0.frame.width - bar.width) < 2
        }
        XCTAssertNotNil(form, "the scrollable form inside the presented sheet")
        for _ in 0..<8 {
            if field.exists && field.isHittable { break }
            form?.swipeUp(velocity: .fast)
        }
        require("task-text").tap()
        field.typeText(text)
        for open in [false, true] {
            resize(open: open)
            _ = require("task-cancel")
            XCTAssertEqual(field.value as? String, text)
            screenshot(open ? "task-draft-unfolded" : "task-draft-folded")
        }
        require("task-cancel").tap()
        let discard = app.buttons["Discard"].firstMatch
        XCTAssertTrue(discard.waitForExistence(timeout: 10))
        discard.tap()
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element("task-cancel"))
        XCTAssertEqual(XCTWaiter().wait(for: [closed], timeout: 10), .completed)
    }
}
