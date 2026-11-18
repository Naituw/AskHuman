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
        // A test that folded from the upright screen leaves the inner display upright.
        turnInnerScreen(portrait: false, waitForApp: false)
        app.launchArguments = ["-AHMock", "YES", "-AHLatency", "0.05", "-AHWebMarkdown", "NO",
                               "-AHDeveloperMode", "NO", "-AHAutoAdvance", "NO", "-AHTab", tab,
                               "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
        XCTAssertGreaterThanOrEqual(app.windows.firstMatch.frame.width, 700)
    }

    private struct FoldState: Decodable {
        let nonce: String
        let activeScreen: String
        let orientation: String?
    }

    private func resize(open: Bool, waitForApp: Bool = true) {
        send(open ? "open" : "close", screen: open ? "inner" : "cover") { [self] in
            guard waitForApp else { return true }
            let width = app.windows.firstMatch.frame.width
            return open ? width >= 700 : width < 600
        }
    }

    /// Turn the inner screen upright or sideways.
    private func turnInnerScreen(portrait: Bool, waitForApp: Bool = true) {
        let orientation = portrait ? "portrait" : "landscape"
        send("rotate-\(orientation)", screen: "inner", orientation: orientation) { [self] in
            guard waitForApp else { return true }
            let frame = app.windows.firstMatch.frame
            return portrait ? frame.height > frame.width : frame.width > frame.height
        }
    }

    /// Ask the host driver for a fold or a turn and wait until it reports the Duo there and the app has followed.
    private func send(_ command: String, screen: String, orientation: String? = nil, appReady: @escaping () -> Bool) {
        let nonce = UUID().uuidString
        let acknowledgement = URL.documentsDirectory.appendingPathComponent("duo-fold-state.json")
        print("AH_DUO_FOLD_COMMAND \(command) \(nonce)")
        fflush(stdout)
        let done = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard let data = try? Data(contentsOf: acknowledgement),
                  let state = try? JSONDecoder().decode(FoldState.self, from: data),
                  state.nonce == nonce, state.activeScreen == screen,
                  orientation == nil || state.orientation == orientation else { return false }
            return appReady()
        }, object: nil)
        XCTAssertEqual(XCTWaiter().wait(for: [done], timeout: 60), .completed)
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

    /// The outer screen puts the tab bar and the buttons of the page in one strip. A request that was open in the
    /// columns is in the new stack from its first frame, which is not how a request is pushed over the list, and it
    /// has to keep every button just the same (the tab bar gives way). Left to the system's own choice, "next" and
    /// the menu fold into an overflow button and all five tabs stay. No draft here: a keyboard moves things around.
    func testFoldingAnOpenRequestKeepsEveryButtonBesideTheTabs() {
        launch()
        require("inbox-row-req-migration").tap()
        _ = require("request-title")
        for cycle in 0..<3 {
            resize(open: false)
            for id in ["BackButton", "previous-request", "next-request", "request-menu"] { _ = require(id) }
            XCTAssertFalse(element("BottomOverflowBarButtonItem").exists, "the buttons are folded into the system's overflow menu")
            screenshot("folded-buttons-\(cycle)")
            if cycle < 2 { resize(open: true) }
        }
    }

    /// The Duo can fold while its inner screen is upright, laid out with the horizontal bars. The strip of the outer
    /// screen then has to be built for a navigation controller that SwiftUI creates in the middle of that change, and
    /// about half of such folds once left it with the tabs only: the page's buttons, the back button among them, were
    /// declared but never put in the strip. Folding at once after the turn, or from the sideways screen, never did.
    func testFoldingFromTheUprightInnerScreenKeepsEveryButtonBesideTheTabs() {
        launch()
        require("inbox-row-req-migration").tap()
        _ = require("request-title")
        for cycle in 0..<4 {
            turnInnerScreen(portrait: true)
            // The failure needs the upright layout to have settled first.
            Thread.sleep(forTimeInterval: 1.2)
            resize(open: false)
            for id in ["BackButton", "previous-request", "next-request", "request-menu"] { _ = require(id) }
            XCTAssertFalse(element("BottomOverflowBarButtonItem").exists, "the buttons are folded into the system's overflow menu")
            screenshot("upright-folded-\(cycle)")
            if cycle < 3 {
                resize(open: true, waitForApp: false)
                turnInnerScreen(portrait: false)
            }
        }
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
