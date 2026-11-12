import XCTest

/// Away mode through the real UI over the mock Macs (`docs/specs/special-replies.md` D30–D36), in English or Chinese
/// (`TEST_RUNNER_AH_UI_LANG=zh-Hans`). The `away` scenario has Macs that know away mode and nothing on; `awayOn` has
/// the Home MacBook Pro away since 40 minutes, one session muted, one asking normally, and the migration question an
/// escalation; the plain demo has Macs from before away mode.
@MainActor
final class AskHumanAwayUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        if let run = testRun, run.totalFailureCount > 0, app != nil { shot("failed-" + name.filter { $0.isLetter }) }
    }

    // MARK: helpers

    @discardableResult
    private func launch(_ scenario: String, tab: String = "agents") -> XCUIApplication {
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", scenario, "-AHTab", tab, "-AHLatency", "0.05", "-AHAutoAdvance", "NO",
            "-AHWebMarkdown", "NO", "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ]
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

    private func gone(_ id: String, timeout: TimeInterval = 10) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element(id))
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    private func localized(_ en: String, _ zh: String) -> String { language == "en" ? en : zh }

    /// Anything on screen whose label is exactly `text` (menu items, chips, buttons).
    private func labelled(_ text: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", text)).firstMatch
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
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func tab(_ en: String, _ zh: String) -> XCUIElement {
        app.tabBars.buttons[localized(en, zh)]
    }

    private func openRequest(_ id: String) {
        requireElement("inbox-row-\(id)").tap()
    }

    private func back() {
        app.navigationBars.buttons.element(boundBy: 0).tap()
    }

    /// After a send the app goes back to the list by itself. When the finish event overtakes the "sending" state (a race
    /// of the mock, whose answers come back at once) the page stays on its "ended" banner, and the way back is the button.
    private func leaveTheSentRequest() {
        if element("filter-menu").waitForExistence(timeout: 6) { return }
        back()
        XCTAssertTrue(element("filter-menu").waitForExistence(timeout: 10), "back at the list")
    }

    /// Swipes until a row is clear of the top bar and the bottom bar. On older systems the automatic scrolling of a tap does
    /// not bring a row out from under the bar, and `isHittable` still says yes for it.
    private func scrollIntoReach(_ id: String, maxSwipes: Int = 10) {
        var swipes = 0
        while swipes < maxSwipes {
            let target = element(id)
            if target.exists, target.frame.minY > 140, target.frame.maxY < app.frame.height - 200 { break }
            app.swipeUp(velocity: .slow)
            swipes += 1
        }
        XCTAssertTrue(element(id).isHittable, "\(id) is within reach")
    }

    /// "Send" takes the place of "Next" once the last question has been on screen.
    private func reachTheEnd(maxSwipes: Int = 10) {
        var swipes = 0
        while !element("send-button").exists, swipes < maxSwipes {
            app.swipeUp(velocity: .slow)
            swipes += 1
        }
    }

    // MARK: Macs from before away mode

    func testMacsFromBeforeAwayModeShowNoControls() {
        launch("demo")
        requireElement("agent-row-\(workingKey)")
        XCTAssertFalse(element("away-moon").exists, "an older Mac would drop what is sent")
        XCTAssertFalse(element("away-strip").exists)
        tab("Inbox", "收件箱").tap()
        openRequest("req-select")
        requireElement("option-0-0")
        XCTAssertFalse(element("quick-reply-menu").exists, "no bar menu for a Mac that has no away mode")
    }

    private let workingKey = "claude:cc-flaky"

    // MARK: turning it on

    func testTheMoonOpensTheStartSheetAndTurnsAwayModeOn() {
        launch("away")
        let moon = requireElement("away-moon")
        XCTAssertEqual(moon.value as? String, localized("Off", "已关闭"))
        XCTAssertFalse(element("away-strip").exists)
        moon.tap()

        requireElement("away-start")
        XCTAssertTrue(requireElement("away-mac-mockhomemacmacmacmacmacmac").exists, "two Macs: the choice is shown")
        XCTAssertTrue(element("away-mac-mockofficemacmacmacmacmac").exists || app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'away-mac-'")).count == 2)
        shot("away-1-start-sheet")

        let note = requireElement("away-note")
        note.tap()
        note.typeText("In a meeting")
        requireElement("away-start").tap()

        let strip = requireElement("away-strip")
        XCTAssertTrue(strip.exists)
        XCTAssertTrue(gone("away-start"), "the sheet closes once the Macs took it")
        XCTAssertEqual(requireElement("away-moon").value as? String, localized("On", "已开启"))
        shot("away-2-strip")
    }

    func testTheStripShowsOnEveryTabAndEndsInOneTap() {
        launch("awayOn")
        requireElement("away-strip")
        shot("away-3-strip-agents")
        for (en, zh) in [("Inbox", "收件箱"), ("Todos", "待办"), ("History", "历史"), ("Macs", "Mac"), ("Agents", "Agents")] {
            tab(en, zh).tap()
            XCTAssertTrue(element("away-strip").waitForExistence(timeout: 5), "the strip on the \(en) tab")
        }
        requireElement("away-end").tap()
        XCTAssertTrue(gone("away-strip"), "ending is one tap, no question")
        XCTAssertEqual(requireElement("away-moon").value as? String, localized("Off", "已关闭"))
    }

    func testTheSolidMoonEndsAwayModeToo() {
        launch("awayOn")
        let moon = requireElement("away-moon")
        XCTAssertEqual(moon.value as? String, localized("On", "已开启"))
        moon.tap()
        XCTAssertTrue(gone("away-strip"))
    }

    func testThePagesPushedOverATabHaveNoStrip() {
        launch("awayOn", tab: "inbox")
        requireElement("away-strip")
        openRequest("req-migration")
        requireElement("escalation-notice")
        XCTAssertFalse(element("away-strip").exists, "the strip belongs to the root list, and the notice has its own way to end it")
    }

    // MARK: escalations

    func testTheEscalatedQuestionCarriesANoticeThatCanEndAwayMode() {
        launch("awayOn", tab: "inbox")
        openRequest("req-migration")
        let notice = requireElement("escalation-notice")
        XCTAssertTrue(notice.exists)
        shot("away-4-escalated-question")
        requireElement("escalation-end-away").tap()
        XCTAssertTrue(gone("escalation-end-away"), "away mode is off: nothing left to end")
        back()
        XCTAssertTrue(gone("away-strip"))
    }

    func testAnOrdinaryQuestionHasNoNotice() {
        launch("awayOn", tab: "inbox")
        openRequest("req-select")
        requireElement("option-0-0")
        XCTAssertFalse(element("escalation-notice").exists)
    }

    // MARK: the rows

    func testTheRowsSayWhatHappensToASession() {
        launch("awayOn")
        let waiting = requireElement("agent-row-cursor:cur-migrate")
        XCTAssertTrue(waiting.label.contains(localized("Escalated while away", "离开期间升级")), waiting.label)
        let muted = requireElement("agent-row-codex:cdx-notes")
        XCTAssertTrue(muted.label.contains(localized("Not using AskHuman", "不再使用 AskHuman")), muted.label)
        shot("away-5-rows")
    }

    func testTheSessionRowSetsOneSessionAtOnce() {
        launch("away")
        requireElement("agent-row-\(workingKey)").tap()
        let menu = requireElement("session-away-menu")
        XCTAssertEqual(menu.label, localized("Normal", "正常"))
        menu.tap()
        labelled(localized("Stop using AskHuman", "不再使用 AskHuman")).tap()
        XCTAssertTrue(
            NSPredicate(format: "label CONTAINS %@", localized("Stopped", "已停用"))
                .evaluate(with: requireElement("session-away-menu")) || labelled(localized("Stopped", "已停用")).waitForExistence(timeout: 5),
            "the row shows the new state at once"
        )
        shot("away-6-session-row")
        back()
        XCTAssertTrue(
            labelled(localized("Not using AskHuman", "不再使用 AskHuman")).waitForExistence(timeout: 5)
                || requireElement("agent-row-\(workingKey)").label.contains(localized("Not using AskHuman", "不再使用 AskHuman"))
        )
    }

    // MARK: the bar menu

    func testTheBarMenuArmsAnInstructionThatWaitsForSendAndTheCrossTakesItBack() {
        launch("away", tab: "inbox")
        openRequest("req-select")
        requireElement("option-0-0").tap()
        requireElement("quick-reply-menu").tap()
        labelled(localized("Away mode", "离开模式")).tap()

        requireElement("directive-panel")
        XCTAssertEqual(requireElement("send-button").label, localized("Send and go away", "发送并离开"))
        shot("away-7-armed")
        requireElement("directive-cancel").tap()
        XCTAssertTrue(gone("directive-panel"))
        XCTAssertEqual(requireElement("send-button").label, localized("Send", "发送"))
        XCTAssertFalse(element("away-strip").exists, "nothing happened: it only takes effect with the answer")
    }

    func testSendingWithAwayModeArmedTurnsItOn() {
        launch("away", tab: "inbox")
        openRequest("req-select")
        requireElement("option-0-0").tap()
        requireElement("quick-reply-menu").tap()
        labelled(localized("Away mode", "离开模式")).tap()
        requireElement("directive-note").tap()
        element("directive-note").typeText("Back at five")
        requireElement("send-button").tap()
        XCTAssertTrue(gone("directive-panel", timeout: 15), "sent: nothing is left to arm")
        leaveTheSentRequest()
        requireElement("away-strip", timeout: 15)
    }

    func testSendingWithMuteArmedStopsTheSession() {
        launch("away", tab: "inbox")
        openRequest("req-migration")
        requireElement("option-0-0")
        scrollIntoReach("option-0-0")
        element("option-0-0").tap()
        requireElement("quick-reply-menu").tap()
        labelled(localized("Stop using AskHuman", "不再使用 AskHuman")).tap()
        XCTAssertEqual(requireElement("directive-panel").exists, true)
        reachTheEnd()
        let send = requireElement("send-button")
        XCTAssertEqual(send.label, localized("Send and stop", "发送并停用"))
        send.tap()
        XCTAssertTrue(gone("inbox-row-req-migration", timeout: 15))
        leaveTheSentRequest()
        tab("Agents", "Agents").tap()
        let row = requireElement("agent-row-cursor:cur-migrate")
        XCTAssertTrue(row.label.contains(localized("Not using AskHuman", "不再使用 AskHuman")), row.label)
    }
    // MARK: quick replies (docs/specs/special-replies.md D18, D30)

    /// What the mock Macs offer (`MockData.quickReplies`); the system gives the menu's entries no identifiers, so they
    /// are found by their text.
    private let firstReply = "Continue"
    private let secondReply = "Use your own judgement, don't wait for me"
    private let lastReply = "None of these fit, try another approach"

    /// A tap outside closes an open menu and goes no further.
    private func dismissTheMenu() {
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.12)).tap()
    }

    /// Answers all three questions of the migration request, so that nothing is left for a quick reply to answer.
    private func answerEveryQuestionOfTheMigration() {
        requireElement("option-0-0")
        for id in ["option-0-0", "option-1-0", "answer-text-2"] {
            scrollIntoReach(id)
            element(id).tap()
            if id.hasPrefix("answer-text") { element(id).typeText("done") }
        }
        // The count follows the typing a moment later on a slow system, and the menu is built from it when it opens.
        let counted = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label CONTAINS %@", "3/3"), object: element("answered-count")
        )
        XCTAssertEqual(XCTWaiter().wait(for: [counted], timeout: 10), .completed, "all three questions count as answered")
    }

    func testTheBarMenuListsTheMacsQuickRepliesBelowTheInstructionsNearestTheThumb() {
        launch("away", tab: "inbox")
        openRequest("req-migration")
        requireElement("quick-reply-menu").tap()
        let first = labelled(firstReply)
        XCTAssertTrue(first.waitForExistence(timeout: 10), "the first quick reply is in the menu")
        let last = labelled(lastReply)
        let away = labelled(localized("Away mode", "离开模式"))
        XCTAssertTrue(last.exists && away.exists)
        XCTAssertGreaterThan(first.frame.minY, last.frame.minY, "the first reply is the one nearest the thumb")
        XCTAssertGreaterThan(last.frame.minY, away.frame.minY, "the instructions sit above the replies")
        shot("away-8-quick-replies")
    }

    func testAQuickReplyAnswersTheQuestionsLeftAndSendsAtOnce() {
        launch("away", tab: "inbox")
        openRequest("req-migration")
        requireElement("option-0-0").tap()
        requireElement("quick-reply-menu").tap()
        let reply = labelled(secondReply)
        XCTAssertTrue(reply.waitForExistence(timeout: 10))
        reply.tap()
        XCTAssertTrue(gone("inbox-row-req-migration", timeout: 15), "one tap sent the form")

        tab("History", "历史").tap()
        requireElement("history-row-req-migration").tap()
        XCTAssertTrue(labelled(secondReply).waitForExistence(timeout: 10), "the questions nobody answered got the reply")
        shot("away-9-quick-reply-history")
    }

    func testTheQuickRepliesAreGreyedOutOnceEveryQuestionHasAnAnswer() {
        launch("away", tab: "inbox")
        openRequest("req-migration")
        requireElement("quick-reply-menu").tap()
        XCTAssertTrue(labelled(firstReply).waitForExistence(timeout: 10))
        XCTAssertTrue(labelled(firstReply).isEnabled, "nothing is answered yet")
        dismissTheMenu()
        XCTAssertTrue(XCTWaiter().wait(for: [XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == false"), object: labelled(firstReply)
        )], timeout: 10) == .completed)

        answerEveryQuestionOfTheMigration()
        requireElement("quick-reply-menu").tap()
        XCTAssertTrue(labelled(firstReply).waitForExistence(timeout: 10))
        XCTAssertFalse(labelled(firstReply).isEnabled, "nothing is left for a reply to answer")
        XCTAssertTrue(labelled(localized("Away mode", "离开模式")).isEnabled, "the instructions stay")
        shot("away-10-replies-greyed")
    }

    func testAStrictChoiceAndWhatsNextHaveNoQuickReplies() {
        launch("away", tab: "inbox")
        for id in ["req-select", "req-next"] {
            openRequest(id)
            requireElement("quick-reply-menu").tap()
            XCTAssertTrue(labelled(localized("Away mode", "离开模式")).waitForExistence(timeout: 10), "\(id) has the instructions")
            XCTAssertFalse(labelled(firstReply).exists, "\(id) takes no free-text reply")
            dismissTheMenu()
            back()
        }
    }
}
