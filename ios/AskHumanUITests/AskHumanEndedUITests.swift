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
    /// name of what answered it).
    @discardableResult
    private func launch(
        _ request: String, how: String = "answeredElsewhere", by: String? = "Popup", after: Double = 6, latency: Double = 0.05
    ) -> XCUIApplication {
        app?.terminate()
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        var arguments = [
            "-AHMock", "YES", "-AHScenario", "demo", "-AHLatency", "\(latency)", "-AHAutoAdvance", "NO", "-AHWebMarkdown", "NO",
            "-AHOpen", request, "-AHEndAfter", "\(after)", "-AHEndHow", how,
            "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ]
        if let by { arguments += ["-AHEndBy", by] }
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
        XCTAssertTrue(found.waitForExistence(timeout: timeout), "\(id) did not appear", file: file, line: line)
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
        XCTAssertTrue(strip.isHittable)
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

    // MARK: not ours to announce

    func testAnAnswerSentFromThisPhoneIsNotAnEnding() {
        // A long wait for the Mac's acknowledgement, so that the form is still there, "sending", when it comes.
        launch("req-select", after: 600, latency: 1.0)
        requireElement("option-0-0").tap()
        requireElement("send-button").tap()
        requireElement("sent-receipt", timeout: 20)
        XCTAssertFalse(element("finished-notice").exists, "the receipt is the page: the request did not end behind our back")
    }
}
