import XCTest

/// "Remove for now" over the mock (docs/specs/ios-offline-remove.md): while a Mac is offline (`-AHLink macOffline`) the
/// request page's line about it offers to take the request off the list, so do the page's menu and a swipe on the row.
@MainActor
final class AskHumanRemoveForNowUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }
    private func localized(_ en: String, _ zh: String) -> String { language == "en" ? en : zh }
    private var removeWords: String { localized("Remove for now", "暂时移除") }

    override func setUp() async throws { continueAfterFailure = false }

    /// `link`: the Macs go to that link at the start (`-AHLink`).
    private func launch(link: String? = "macOffline") {
        app = XCUIApplication()
        app.launchArguments = ["-AHMock", "YES", "-AHScenario", "demo", "-AHLatency", "0.05", "-AHAutoAdvance", "NO",
                               "-AHWebMarkdown", "NO",
                               "-AppleLanguages", "(\(language))", "-AppleLocale", language == "en" ? "en_US" : "zh_CN"]
            + (link.map { ["-AHLink", $0] } ?? [])
        app.launch()
    }

    private func element(_ id: String) -> XCUIElement { app.descendants(matching: .any).matching(identifier: id).firstMatch }

    @discardableResult
    private func require(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let item = element(id)
        XCTAssertTrue(item.waitForExistence(timeout: timeout), "missing \(id)", file: file, line: line)
        return item
    }

    private func gone(_ id: String, timeout: TimeInterval = 10) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element(id))
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"], !folder.isEmpty {
            let url = URL(fileURLWithPath: folder).appendingPathComponent("\(language)-\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
            return
        }
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = "\(language)-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// The dialog's buttons. Before iOS 26 it is an action sheet on a phone; from iOS 26 a popover, which has no cancel button.
    private func dialogButton(_ title: String) -> XCUIElement { app.sheets.firstMatch.buttons[title] }

    /// Leaves the dialog without choosing: its cancel button where it has one, else a tap beside the popover.
    private func keepIt() {
        let keep = dialogButton(localized("Keep it", "保留"))
        if keep.waitForExistence(timeout: 1) {
            keep.tap()
        } else {
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.72)).tap()
        }
    }

    // MARK: the line on the bar of a page

    func testTheOfflineLineOfAQuestionOffersRemovalAndTheDialogCanKeepIt() {
        launch()
        require("inbox-row-req-select").tap()
        let chip = require("hint-remove")
        XCTAssertTrue(chip.label.contains(removeWords), "the line says what it offers: \(chip.label)")
        XCTAssertFalse(element("send-button").isEnabled, "sending stays out of reach while the Mac is away")
        shot("remove-chip-ask")

        chip.tap()
        XCTAssertTrue(dialogButton(removeWords).waitForExistence(timeout: 5), "the dialog asks first")
        shot("remove-dialog")
        keepIt()

        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: dialogButton(removeWords))
        XCTAssertEqual(XCTWaiter().wait(for: [closed], timeout: 5), .completed, "the dialog is closed")
        XCTAssertTrue(element("hint-remove").exists, "nothing changed: the page is still there")
        XCTAssertTrue(element("send-button").exists)
    }

    func testRemovingFromTheLineGoesBackToTheListWithoutTheRequest() {
        launch()
        require("inbox-row-req-select").tap()
        require("hint-remove").tap()
        dialogButton(removeWords).tap()

        require("inbox-row-req-migration")
        XCTAssertTrue(gone("inbox-row-req-select"), "the request is off the list")
        XCTAssertFalse(element("send-button").exists, "the page gave way to the list")
        shot("removed-list")
    }

    func testTheLineOfAConfirmationOffersRemovalToo() {
        launch()
        require("inbox-row-req-permission").tap()
        let chip = require("hint-remove")
        XCTAssertTrue(chip.label.contains(removeWords), chip.label)
        XCTAssertFalse(element("submit-button").isEnabled)
        shot("remove-chip-confirm")
        chip.tap()
        dialogButton(removeWords).tap()
        XCTAssertTrue(gone("inbox-row-req-permission"))
    }

    // MARK: the menu of a page

    func testTheMenuOffersRemovalInsteadOfACancellationThatCannotBeSent() {
        launch()
        require("inbox-row-req-select").tap()
        require("request-menu").tap()
        let remove = require("request-menu-remove", timeout: 5)
        XCTAssertEqual(remove.label, removeWords)
        XCTAssertFalse(app.buttons[localized("Cancel request", "取消请求")].exists, "there is no cancelling an offline Mac")
        shot("remove-menu")
        remove.tap()
        XCTAssertTrue(dialogButton(removeWords).waitForExistence(timeout: 5), "the same dialog")
        dialogButton(removeWords).tap()
        XCTAssertTrue(gone("inbox-row-req-select"))
    }

    // MARK: the list

    func testARowCanBeSwipedAwayWhileItsMacIsOffline() {
        launch()
        let row = require("inbox-row-req-next")
        row.swipeLeft()
        // A long swipe takes the request away by itself; a short one shows the button.
        let action = element("swipe-remove-req-next")
        if action.waitForExistence(timeout: 2) {
            shot("remove-swipe")
            action.tap()
        }
        XCTAssertTrue(gone("inbox-row-req-next"))
        require("inbox-row-req-migration")
    }

    // MARK: only where nothing else can be done

    func testNothingIsOfferedWhileTheMacIsReachable() {
        launch(link: nil)
        require("inbox-row-req-select").tap()
        _ = require("send-button")
        XCTAssertFalse(element("hint-remove").exists)
        require("request-menu").tap()
        XCTAssertTrue(app.buttons[localized("Cancel request", "取消请求")].firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(element("request-menu-remove").exists)
        // Leave the menu, then the page (its back button is the bar's first; only from iOS 26 is it named `BackButton`):
        // the list has no swipe action for a row whose Mac can be reached.
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.72)).tap()
        app.navigationBars.buttons.element(boundBy: 0).tap()
        let row = require("inbox-row-req-select")
        row.swipeLeft()
        XCTAssertFalse(element("swipe-remove-req-select").waitForExistence(timeout: 2))
    }

    func testNothingIsOfferedWhileTheConnectionIsOnItsWay() {
        for link in ["connecting", "brokerDown"] {
            launch(link: link)
            require("inbox-row-req-select").tap()
            XCTAssertTrue(element("request-connection").waitForExistence(timeout: 10), "the line about the link is there")
            XCTAssertFalse(element("hint-remove").exists, "but it has no way out for \(link)")
            app.terminate()
        }
    }
}
