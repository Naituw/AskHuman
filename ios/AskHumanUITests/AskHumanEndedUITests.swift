import XCTest

/// A request that ends somewhere else while its page is open (`docs/specs/ios-answer-flow.md` §3.7), through the real UI
/// over the mock Macs, in English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`): a strip under the navigation bar says
/// nothing is asked any more, the controls stop taking answers, and the strip stays in view when the page is scrolled.
/// The mock ends the request a few seconds after its page has opened (`-AHEndAfter`, `-AHEndHow`, `-AHEndBy`).
@MainActor
final class AskHumanEndedUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        if let run = testRun, run.totalFailureCount > 0, app != nil { shot("failed-" + name.filter { $0.isLetter }) }
    }

    // MARK: helpers

    /// Opens `request` and has the mock Macs end it `after` seconds later, the way a Mac reports `how` (and `by`, the
    /// name of what answered it). Moving on by itself is off unless `autoAdvance` is on, and then the strip counts down
    /// `countdown` seconds (the app's own 3 is short for a test that has to look and press).
    @discardableResult
    private func launch(
        _ request: String, how: String = "answeredElsewhere", by: String? = "Popup", after: Double = 6, latency: Double = 0.05,
        scenario: String = "demo", autoAdvance: Bool = false, countdown: Double? = nil, contentSize: String? = nil,
        endInBackground: Bool = false, receiptHold: Double? = nil
    ) -> XCUIApplication {
        app?.terminate()
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        var arguments = [
            "-AHMock", "YES", "-AHScenario", scenario, "-AHLatency", "\(latency)", "-AHAutoAdvance", autoAdvance ? "YES" : "NO",
            "-AHWebMarkdown", "NO",
            "-AHOpen", request, "-AHEndAfter", "\(after)", "-AHEndHow", how,
            "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ]
        if let by { arguments += ["-AHEndBy", by] }
        if let countdown { arguments += ["-AHEndedCountdown", "\(countdown)"] }
        if endInBackground { arguments += ["-AHEndInBackground", "YES"] }
        if let receiptHold { arguments += ["-AHAdvanceHold", "\(receiptHold)"] }
        if let contentSize { arguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategory" + contentSize] }
        app.launchArguments = arguments
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
        let exists = found.waitForExistence(timeout: timeout)
        if !exists { add(XCTAttachment(string: app.debugDescription)) }
        XCTAssertTrue(exists, "\(id) did not appear", file: file, line: line)
        return found
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

    /// The strip's spoken text: what to do, then why.
    private func stripLabel() -> String { element("finished-notice").label }

    /// The strip slides in and the body fades over about a third of a second: a screenshot waits for them.
    private func settle() { Thread.sleep(forTimeInterval: 1) }

    // MARK: a question

    func testAQuestionThatEndsWhileOpenStopsTakingAnswers() {
        launch("req-select", after: 10)
        let option = requireElement("option-0-0")
        XCTAssertTrue(option.isEnabled, "while it is open, the options take a choice")
        XCTAssertFalse(element("finished-notice").exists, "no strip on a request that is still open")
        shot("ask-open")

        requireElement("finished-notice", timeout: 40)
        let label = stripLabel()
        XCTAssertTrue(label.contains(localized("No reply needed", "无需回复")), "the strip says no reply is needed: \(label)")
        XCTAssertTrue(label.contains("Popup"), "and where it was answered: \(label)")
        XCTAssertFalse(option.isEnabled, "once it has ended, the options take nothing")
        XCTAssertFalse(element("send-button").isEnabled, "and nothing can be sent")
        settle()
        shot("ask-ended")
    }

    func testTheStripStaysInViewWhenThePageIsScrolled() {
        launch("req-migration", after: 3)
        requireElement("finished-notice", timeout: 40)
        settle()
        let top = element("finished-notice").frame.minY
        for _ in 0..<3 { app.swipeUp() }
        let strip = element("finished-notice")
        XCTAssertTrue(strip.exists)
        XCTAssertEqual(strip.frame.minY, top, accuracy: 2, "the strip is pinned under the bar, not scrolled away with the page")
        // The sentence is a static text, and the page's scroll view comes after it in the accessibility order: what has
        // scrolled behind it counts as in front, so its own hit test says nothing. The button is what one taps.
        XCTAssertTrue(element("finished-action").isHittable, "and what it offers can still be tapped")
        shot("ask-ended-scrolled")
    }

    func testEachWayARequestEndsHasItsOwnWords() {
        let cases: [(how: String, by: String?, en: String, zh: String)] = [
            ("answeredElsewhere", nil, "It was answered elsewhere.", "已在别处回答。"),
            ("cancelled", nil, "The Agent cancelled it.", "Agent 已取消它。"),
            ("expired", nil, "It timed out.", "已超时。"),
            ("ended", nil, "It finished while you were away.", "它在你离开期间已结束。"),
        ]
        for item in cases {
            launch("req-select", how: item.how, by: item.by, after: 3)
            requireElement("finished-notice", timeout: 40)
            XCTAssertTrue(stripLabel().contains(localized(item.en, item.zh)), "\(item.how): \(stripLabel())")
            settle()
            shot("ask-ended-\(item.how)")
        }
    }

    // MARK: a confirmation

    func testAConfirmationThatEndsWhileOpenStopsTakingAnswers() {
        launch("req-permission", after: 10)
        let choice = requireElement("choice-0")
        XCTAssertTrue(choice.isEnabled)
        let expiry = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", localized("Expires in", "后过期"))).firstMatch
        XCTAssertTrue(expiry.waitForExistence(timeout: 10), "while it is open, it counts down to its expiry")
        shot("confirm-open")

        requireElement("finished-notice", timeout: 40)
        XCTAssertFalse(choice.isEnabled, "once it has ended, the choices take nothing")
        XCTAssertFalse(element("submit-button").isEnabled)
        XCTAssertFalse(expiry.exists, "and its countdown, which means nothing now, is gone")
        settle()
        shot("confirm-ended")
    }

    // MARK: the way on (§3.8)

    private func gone(_ id: String, timeout: TimeInterval = 20) -> Bool {
        let predicate = NSPredicate(format: "exists == false")
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: element(id))
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    /// A request that is still asking is on screen: its first option (or choice) takes an answer.
    private func aLiveRequestIsUp() -> Bool {
        let option = element("option-0-0"), choice = element("choice-0")
        return (option.exists && option.isEnabled) || (choice.exists && choice.isEnabled)
    }

    /// The Duo runtime can ignore XCTest's hardware Home key. Activating another app changes the real app state.
    private func backgroundApp() {
        let other = XCUIApplication(bundleIdentifier: "com.apple.Preferences")
        other.activate()
        let backgrounded = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            self.app.state == .runningBackground || self.app.state == .runningBackgroundSuspended
        }, object: app)
        XCTAssertEqual(XCTWaiter().wait(for: [backgrounded], timeout: 10), .completed,
                       "AskHuman state \(app.state.rawValue), foreground app state \(other.state.rawValue)")
    }

    func testTheStripCountsDownAndMovesOnToTheNextRequestByItself() {
        launch("req-select", after: 3, autoAdvance: true, countdown: 6)
        requireElement("finished-notice", timeout: 40)
        let action = requireElement("finished-action", timeout: 5)
        XCTAssertEqual(action.label, localized("Next request", "下一条消息"), "another request waits: the button leads to it")
        requireElement("finished-stay", timeout: 5)
        settle()
        shot("ended-countdown")
        XCTAssertTrue(gone("finished-notice", timeout: 30), "when the countdown is over the page moves on by itself")
        XCTAssertTrue(aLiveRequestIsUp(), "to a request that still asks")
    }

    func testACancelledConfirmationCountsDownWhenItsStripIsSeen() {
        launch("req-permission", how: "cancelled", by: nil, after: 5, autoAdvance: true, countdown: 6)
        requireElement("finished-stay", timeout: 30)
        XCTAssertTrue(stripLabel().contains(localized("The Agent cancelled it.", "Agent 已取消它。")))
        XCTAssertTrue(gone("finished-notice", timeout: 20))
        XCTAssertTrue(aLiveRequestIsUp())
    }

    func testACancellationInTheBackgroundWaitsUntilItsStripIsSeen() {
        launch("req-select", how: "cancelled", by: nil, after: 3, autoAdvance: true, countdown: 10, endInBackground: true)
        requireElement("option-0-0")
        backgroundApp()
        // Longer than the old event-based countdown: it would already advance when the app returns.
        Thread.sleep(forTimeInterval: 18)
        app.activate()
        requireElement("finished-notice", timeout: 10)
        requireElement("finished-stay", timeout: 5)
        XCTAssertTrue(stripLabel().contains(localized("The Agent cancelled it.", "Agent 已取消它。")))
        XCTAssertTrue(gone("finished-notice", timeout: 25), "the visible countdown now advances normally")
        XCTAssertTrue(aLiveRequestIsUp())
    }

    func testBackgroundingAfterSeeingTheCountdownDoesNotRestartIt() {
        launch("req-select", how: "cancelled", by: nil, after: 5, autoAdvance: true, countdown: 20)
        requireElement("finished-stay", timeout: 30)
        backgroundApp()
        Thread.sleep(forTimeInterval: 25)
        app.activate()
        XCTAssertTrue(gone("finished-notice", timeout: 6), "the original deadline elapsed while the app was away")
        XCTAssertTrue(aLiveRequestIsUp(), "returning must not start another twenty seconds")
    }

    func testTheButtonMovesOnAtOnce() {
        launch("req-select", after: 3, autoAdvance: true, countdown: 120)
        requireElement("finished-notice", timeout: 40)
        requireElement("finished-action").tap()
        XCTAssertTrue(gone("finished-notice", timeout: 10), "no waiting for the countdown")
        XCTAssertTrue(aLiveRequestIsUp())
    }

    func testStayKeepsThePageAndTheButtonStays() {
        launch("req-select", after: 3, autoAdvance: true, countdown: 5)
        requireElement("finished-notice", timeout: 40)
        requireElement("finished-stay").tap()
        XCTAssertTrue(gone("finished-stay", timeout: 5), "the stay button goes once it has been used")
        Thread.sleep(forTimeInterval: 7)
        XCTAssertTrue(element("finished-notice").exists, "the page is still the ended one, well after the countdown would have run out")
        XCTAssertFalse(aLiveRequestIsUp())
        XCTAssertEqual(element("finished-action").label, localized("Next request", "下一条消息"))
        settle()
        shot("ended-stayed")
        element("finished-action").tap()
        XCTAssertTrue(gone("finished-notice", timeout: 10), "the button still works")
        XCTAssertTrue(aLiveRequestIsUp())
    }

    func testWithNothingElseWaitingTheButtonClosesThePage() {
        launch("req-select", after: 3, scenario: "single", autoAdvance: true, countdown: 6)
        requireElement("finished-notice", timeout: 40)
        XCTAssertEqual(requireElement("finished-action").label, localized("Close", "关闭"), "nothing else waits: the button closes")
        settle()
        shot("ended-close")
        XCTAssertTrue(gone("finished-notice", timeout: 30), "the countdown closes the page")
        XCTAssertTrue(
            app.staticTexts[localized("All caught up", "已全部处理")].waitForExistence(timeout: 10), "and the list is back, with nothing in it"
        )
    }

    func testWithAutoAdvanceOffThereIsOnlyTheButton() {
        launch("req-select", after: 3, autoAdvance: false, countdown: 3)
        requireElement("finished-notice", timeout: 40)
        XCTAssertTrue(requireElement("finished-action").exists)
        XCTAssertFalse(element("finished-stay").exists, "no countdown, so nothing to stay from")
        Thread.sleep(forTimeInterval: 6)
        XCTAssertTrue(element("finished-notice").exists, "and the page is not moved on")
        settle()
        shot("ended-button-only")
        element("finished-action").tap()
        XCTAssertTrue(gone("finished-notice", timeout: 10))
        XCTAssertTrue(aLiveRequestIsUp())
    }

    func testASheetOverThePageStopsTheCountdown() {
        launch("req-select", after: 6, autoAdvance: true, countdown: 4)
        requireElement("option-0-0")
        requireElement("request-menu").tap()
        requireElement("request-menu-add-todo").tap()
        requireElement("request-todo-field")
        // The request ends behind the sheet, and the countdown would have run out well before this.
        Thread.sleep(forTimeInterval: 16)
        requireElement("request-todo-cancel").tap()
        requireElement("finished-notice", timeout: 10)
        XCTAssertFalse(aLiveRequestIsUp(), "the page was not moved on from under the sheet")
        XCTAssertTrue(element("finished-action").exists, "the button is still there")
        XCTAssertFalse(element("finished-stay").exists, "and the countdown is over for good")
        settle()
        shot("ended-after-sheet")
    }

    /// With both buttons on the strip (the countdown is running), at the sizes that crowd it: the buttons are whole and
    /// on screen, none lies on another or on the sentence, and the strip does not take the screen from the page. From the
    /// first accessibility size the strip itself stops growing and stacks the buttons under the sentence.
    func testTheStripHoldsAtLargeTextSizes() {
        for size in ["XXXL", "AccessibilityM", "AccessibilityXXXL"] {
            launch("req-select", how: "answeredElsewhere", by: "Popup", after: 3, autoAdvance: true, countdown: 120, contentSize: size)
            requireElement("finished-notice", timeout: 40)
            let stay = requireElement("finished-stay", timeout: 5), action = requireElement("finished-action", timeout: 5)
            settle()
            let window = app.windows.firstMatch.frame
            let notice = element("finished-notice").frame, stayFrame = stay.frame, actionFrame = action.frame
            // The frames are read one after the other, and the countdown's digits change in between: that moves the
            // buttons by a point or two, and they are only two points apart.
            func apart(_ a: CGRect, _ b: CGRect) -> Bool { !a.insetBy(dx: 6, dy: 0).intersects(b.insetBy(dx: 6, dy: 0)) }
            for (name, frame) in [("Stay", stayFrame), ("action", actionFrame)] {
                XCTAssertGreaterThanOrEqual(frame.minX, 0, "\(size): \(name) is on screen")
                XCTAssertLessThanOrEqual(frame.maxX, window.width, "\(size): \(name) is on screen")
                XCTAssertGreaterThanOrEqual(frame.height, 40, "\(size): \(name) can be hit with a finger")
                XCTAssertTrue(apart(frame, notice), "\(size): \(name) does not lie on the sentence")
            }
            XCTAssertTrue(apart(stayFrame, actionFrame), "\(size): the buttons do not lie on each other")
            let height = stayFrame.union(actionFrame).union(notice).height
            XCTAssertLessThan(height, window.height * 0.2, "\(size): the strip leaves most of the screen to the page (it is \(height) of \(window.height))")
            XCTAssertGreaterThanOrEqual(notice.width, 120, "\(size): the sentence is not left a few words wide (\(notice.width))")
            shot("ended-size-\(size)")
        }
    }

    func testAnAnswerBeingWrittenIsNotMovedAwayWhenItsRequestEnds() {
        // The request ends while an answer is typed: its form stops taking input a moment later, and the keyboard goes
        // with it. The page must not count down over what was written all the same.
        launch("req-migration", after: 30, autoAdvance: true, countdown: 3)
        let field = requireElement("answer-text-0")
        field.tap()
        XCTAssertFalse(element("finished-notice").exists, "the request ended before the typing began: the machine is too slow for this test")
        field.typeText("Half an answer")
        requireElement("finished-notice", timeout: 60)
        XCTAssertFalse(element("finished-stay").exists, "no countdown was started over what is being written")
        Thread.sleep(forTimeInterval: 7)
        XCTAssertTrue(element("finished-notice").exists, "and the page was not moved on, well after the countdown would have run out")
        XCTAssertFalse(aLiveRequestIsUp())
        XCTAssertEqual(element("answer-text-0").value as? String, "Half an answer", "what was written is there to read, and to copy")
        settle()
        shot("ended-while-typing")
    }

    func testTheSettingThatMovesOnSaysItDoesSoInBothCases() {
        app?.terminate()
        let app = XCUIApplication()
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", "demo", "-AHTab", "macs", "-AHLatency", "0.05",
            "-AppleLanguages", "(\(language))", "-AppleLocale", language == "en" ? "en_US" : "zh_CN",
        ]
        app.launch()
        self.app = app
        let toggle = requireElement("auto-advance-toggle")
        for _ in 0..<4 where !toggle.isHittable { app.swipeUp() }
        let title = localized("Open the next request automatically", "自动打开下一个请求")
        let line = localized(
            "Opens it after you send an answer, and a few seconds after the request you are reading ends somewhere else.",
            "发送回答后会自动打开；你正在看的请求在别处结束时，几秒后也会自动打开。"
        )
        XCTAssertTrue(toggle.label.contains(title), "the switch is named for both cases: \(toggle.label)")
        XCTAssertTrue(toggle.label.contains(line), "and the line under it says what it does: \(toggle.label)")
        settle()
        shot("setting-auto-advance")
    }

    // MARK: not ours to announce

    func testAnAnswerSentFromThisPhoneIsNotAnEnding() {
        // A long wait for the Mac's acknowledgement, so that the form is still there, "sending", when it comes.
        launch("req-select", after: 600, latency: 1.0, receiptHold: 5)
        requireElement("option-0-0").tap()
        requireElement("send-button").tap()
        requireElement("sent-receipt", timeout: 20)
        XCTAssertFalse(element("finished-notice").exists, "the receipt is the page: the request did not end behind our back")
    }
}
