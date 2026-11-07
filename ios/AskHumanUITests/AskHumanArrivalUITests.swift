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

    /// The window is wide enough for the inbox to be a column beside the request (the large-screen layout).
    private var isWide: Bool { app.windows.firstMatch.frame.width >= 700 }

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
        // A wide window has its tab bar at the top, not along the bottom edge: the bubbles land at the title.
        status(isWide ? "destination=title" : "destination=inbox")
        status("key=source")
        XCTAssertFalse(element("banner").exists)
        shot("140-arrival-concurrent")
        if !isWide {
            let tab = app.tabBars.buttons[language == "en" ? "Todos" : "待办"]
            XCTAssertTrue(tab.exists)
            tab.tap()
            XCTAssertTrue(tab.isSelected, "the decorative window must pass touches to the source tab bar")
        }
        status("touch=pass")
    }

    func testALiveRequestStillBouncesWhileTheInboxIsFrontmost() {
        launch(extra: ["-AHTab", "inbox"])
        if !isWide {
            let inbox = app.tabBars.buttons[language == "en" ? "Inbox" : "收件箱"]
            XCTAssertTrue(inbox.isSelected)
        }
        status("active=1")
        status("phase=bounce")
        // Where the list is a column of the window the bubble is for its row, once the row is there.
        status(isWide ? "destination=row" : "destination=inbox", timeout: 20)
        shot("147-arrival-live-inbox")
    }

    /// In a window wide enough for the list to be a column beside the request, a new request flies to the unread dot of
    /// its own row, as the Mac's flies to its sidebar's, and the dot waits for it. The phone keeps the tab as its target.
    func testInAWideWindowARequestFliesToTheUnreadDotOfItsRow() throws {
        launch(extra: ["-AHTab", "inbox"], slowdown: 10)
        try XCTSkipUnless(isWide, "The list is a column of the window only where the window is wide.")
        let dot = NSPredicate(format: "identifier BEGINSWITH 'inbox-row-req-live-'")
        let row = app.descendants(matching: .any).matching(dot).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 30), "the new request has a row in the list")
        status("phase=flight", timeout: 60)
        status("destination=row")
        let value = require("arrival-motion-state").value as? String ?? ""
        let aim = value.components(separatedBy: ";").first { $0.hasPrefix("target=") }
            .map { $0.dropFirst("target=".count).split(separator: ",").compactMap { Double($0) } } ?? []
        XCTAssertEqual(aim.count, 2, value)
        if aim.count == 2 {
            let frame = row.frame
            XCTAssertGreaterThanOrEqual(aim[0], frame.minX - 4, "the bubble aims at the leading edge of its row: \(value) \(frame)")
            XCTAssertLessThanOrEqual(aim[0], frame.minX + 48, "the bubble aims at the leading edge of its row: \(value) \(frame)")
            XCTAssertGreaterThanOrEqual(aim[1], frame.minY, "the bubble aims at the dot of its row: \(value) \(frame)")
            XCTAssertLessThanOrEqual(aim[1], frame.minY + 44, "the bubble aims at the dot of its row: \(value) \(frame)")
        }
        shot("148-arrival-row-flight")
        status("phase=landed", timeout: 40)
        shot("149-arrival-row-landed")
        // The decorative window goes away with the last bubble, and takes its diagnostic with it.
        let over = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element("arrival-motion-state"))
        XCTAssertEqual(XCTWaiter().wait(for: [over], timeout: 40), .completed, "the animation ends")
        XCTAssertTrue(row.exists, "the request keeps its row")
        shot("150-arrival-row-done")
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
        // The cover goes away with its own animation, which a busy machine stretches.
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element("arrival-fullscreen-fixture"))
        XCTAssertEqual(XCTWaiter().wait(for: [closed], timeout: 10), .completed, "the full-screen page closes")
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
        // The keys reach the field's accessibility value a moment after they are synthesized; a busy machine stretches that.
        let typed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS 'done'"), object: field)
        XCTAssertEqual(XCTWaiter().wait(for: [typed], timeout: 6), .completed, "the typed text arrives; the field holds \(String(describing: field.value))")
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
        // A drag across most of a page, not `swipeLeft()`: on a machine that was busy with other work the swipe did not turn
        // the page (every time, with or without this change), the drag, which turns it by how far it went, did. Even the drag
        // is lost now and then on such a machine (also on the code from before the arrival rework), so a second one is allowed
        // when the page has not moved at all; a bubble that blocked touches would lose both.
        var paged = XCTWaiter.Result.timedOut
        var attempts = 0
        while attempts < 2, paged != .completed, require("gallery-name").label == "wide-1.png" {
            attempts += 1
            gallery.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5))
                .press(forDuration: 0.05, thenDragTo: gallery.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.5)))
            let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "tall-2.png"), object: require("gallery-name"))
            paged = XCTWaiter().wait(for: [expectation], timeout: 5)
        }
        XCTAssertEqual(paged, .completed, "the viewer pages under the bubble (\(attempts) drags): \(require("gallery-name").label), \(element("gallery-position").label), \(element("arrival-motion-state").value ?? "")")
        status("key=source")
    }
}
