import XCTest

/// End-to-end flows through the real UI over the mock backend (`-AHMock YES`), in English or Chinese
/// (`TEST_RUNNER_AH_UI_LANG=zh-Hans`). The screenshots taken on the way are attached to the result.
@MainActor
final class AskHumanUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    private let homeMac = "mockhomemacmacmacmacmacmac"
    private let officeMac = "mockofficemacmacmacmacmac"

    private let workingAgent = "claude:cc-flaky"
    private let waitingAgent = "cursor:cur-migrate"
    private let idleAgent = "codex:cdx-notes"
    private let endedAgent = "cursor:cur-readme"
    private let officeAgent = "claude:cc-users"
    private let unsupportedAgent = "codex:cdx-leak"

    override func setUp() async throws {
        continueAfterFailure = false
    }

    // MARK: helpers

    @discardableResult
    private func launch(
        _ scenario: String = "demo", latency: Double = 0.05, autoAdvance: Bool = false, extra: [String] = []
    ) -> XCUIApplication {
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", scenario, "-AHLatency", "\(latency)", "-AHAutoAdvance", autoAdvance ? "YES" : "NO",
            "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ] + extra
        app.launch()
        self.app = app
        return app
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func requireElement(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let found = element(id)
        XCTAssertTrue(found.waitForExistence(timeout: timeout), "\(id) did not appear", file: file, line: line)
        return found
    }

    /// Saves a screenshot: into the folder named by `AH_UI_SHOTS` when set (cheap, easy to browse),
    /// else as an attachment of the test result.
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

    private func openRequest(_ id: String) {
        requireElement("inbox-row-\(id)").tap()
    }

    private func gone(_ id: String, timeout: TimeInterval = 10) -> Bool {
        let predicate = NSPredicate(format: "exists == false")
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: element(id))
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    /// Scrolls the form until the element sits in the visible area, clear of the bars at either end.
    private func reveal(_ id: String) {
        // A lazy form only creates the rows near the visible area: scroll until the element exists.
        var tries = 0
        while !element(id).exists, tries < 8 {
            app.swipeUp(velocity: .slow)
            tries += 1
        }
        let target = requireElement(id)
        let screen = app.windows.firstMatch.frame
        for _ in 0..<10 {
            let frame = target.frame
            if frame.minY > 120, frame.maxY < screen.height - 220 { return }
            if frame.minY <= 120 {
                app.swipeDown(velocity: .slow)
            } else {
                app.swipeUp(velocity: .slow)
            }
        }
    }

    /// Opens the filter menu of the tab and picks the entry with this title. Menu entries are found by
    /// their title, as the system gives them no identifiers.
    private func chooseFilter(_ title: String) {
        requireElement("filter-menu").tap()
        let entry = app.buttons.matching(NSPredicate(format: "label == %@", title)).firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 5), "the filter menu has no entry \(title)")
        entry.tap()
    }

    private func tapOption(_ id: String) {
        reveal(id)
        requireElement(id).tap()
    }

    private func type(_ text: String, into id: String) {
        reveal(id)
        let field = requireElement(id)
        field.tap()
        field.typeText(text)
    }

    /// Waits until the element's accessibility label equals `label`.
    private func waitForLabel(_ id: String, _ label: String, timeout: TimeInterval = 10) -> Bool {
        let predicate = NSPredicate(format: "label == %@", label)
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: element(id))
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    private func localized(_ en: String, _ zh: String) -> String { language == "en" ? en : zh }

    private func answeredCount() -> String {
        requireElement("answered-count").label
    }

    private func tab(_ index: Int) {
        let button = app.tabBars.buttons.element(boundBy: index)
        XCTAssertTrue(button.waitForExistence(timeout: 10))
        button.tap()
    }

    // MARK: inbox

    func testInboxListsEveryPendingRequestAndFilters() {
        launch()
        for id in ["req-permission", "req-migration", "req-select", "req-next", "req-stop"] {
            _ = requireElement("inbox-row-\(id)")
        }
        XCTAssertFalse(element("filter-note").exists, "an unfiltered list gives the filter no room")
        shot("01-inbox")

        chooseFilter(localized("Confirmations", "确认"))
        XCTAssertTrue(gone("inbox-row-req-migration"))
        XCTAssertTrue(element("inbox-row-req-permission").exists)
        XCTAssertTrue(element("inbox-row-req-stop").exists)
        _ = requireElement("filter-note")
        shot("02-inbox-filtered")

        chooseFilter("Office iMac")
        XCTAssertTrue(gone("inbox-row-req-permission"))
        XCTAssertTrue(element("inbox-row-req-stop").exists)
        XCTAssertTrue(element("inbox-row-req-select").exists)

        requireElement("filter-clear").tap()
        _ = requireElement("inbox-row-req-migration")
        XCTAssertTrue(gone("filter-note"), "clearing the filter gives its room back")
    }

    func testTheHeaderOfTheListsIsAnOrdinaryNavigationBar() {
        launch()
        _ = requireElement("inbox-row-req-migration")
        // The title bar of iOS 26 is 54 pt high; the old pinned filter strip made it three times that.
        let bar = app.navigationBars.firstMatch
        XCTAssertLessThan(bar.frame.height, 70, "bar is \(bar.frame)")
        let first = requireElement("inbox-row-req-permission")
        XCTAssertLessThan(first.frame.minY, bar.frame.maxY + 60, "the list starts right under the bar")
        tab(1)
        _ = requireElement("agent-row-\(workingAgent)")
        XCTAssertLessThan(app.navigationBars.firstMatch.frame.height, 70)
    }

    // MARK: questions

    func testAnsweringAMultiQuestionRequestSendsItAndLeavesTheInbox() {
        launch()
        openRequest("req-migration")
        // Several questions: the bar leads through them first; Send comes with the last one.
        _ = requireElement("next-button")
        XCTAssertFalse(element("send-button").exists, "nothing was read, nothing can be sent")
        XCTAssertTrue(answeredCount().contains("0/3"))
        shot("10-answer-top")

        tapOption("option-0-0")
        XCTAssertTrue(answeredCount().contains("1/3"))
        tapOption("option-1-0")
        tapOption("option-1-2")
        XCTAssertTrue(answeredCount().contains("2/3"))
        type("Keep the rollout behind a flag.", into: "answer-text-2")
        XCTAssertTrue(answeredCount().contains("3/3"))
        shot("11-answer-filled")

        let send = requireElement("send-button")
        XCTAssertTrue(send.isEnabled, "a request that is not select-only may be sent as it is")
        send.tap()
        XCTAssertTrue(gone("inbox-row-req-migration", timeout: 15), "the answered request should leave the inbox")
        _ = requireElement("inbox-row-req-select")
        shot("12-after-send")
    }

    func testSendingWaitsUntilTheLastQuestionWasSeen() {
        launch()
        openRequest("req-migration")
        var taps = 0
        // Nothing is answered: only looking at the questions brings Send up.
        while !element("send-button").exists, taps < 5 {
            let next = requireElement("next-button")
            XCTAssertTrue(next.isHittable)
            if taps == 0 { shot("16-next") }
            next.tap()
            taps += 1
            Thread.sleep(forTimeInterval: 0.7)
        }
        XCTAssertTrue(element("send-button").waitForExistence(timeout: 5), "Send should show once the last question was seen")
        XCTAssertTrue((2...3).contains(taps), "three questions need two or three taps, took \(taps)")
        XCTAssertFalse(element("next-button").exists)
        shot("17-send-after-next")
        XCTAssertTrue(requireElement("send-button").isEnabled)
        XCTAssertTrue(answeredCount().contains("0/3"), "looking is not answering")
    }

    func testScrollingToTheLastQuestionBringsSendUp() {
        launch()
        openRequest("req-migration")
        _ = requireElement("next-button")
        reveal("answer-text-2")
        XCTAssertTrue(element("send-button").waitForExistence(timeout: 5), "Send should show once the last question is on screen")
        XCTAssertFalse(element("next-button").exists)
    }

    func testAnAnsweredLastQuestionBringsSendUpAtOnce() {
        launch()
        openRequest("req-migration")
        _ = requireElement("next-button")
        type("Straight to the last one.", into: "answer-text-2")
        XCTAssertTrue(element("send-button").waitForExistence(timeout: 5))
    }

    func testChoosingNothingElseIsEnoughForASelectOnlyQuestion() {
        launch()
        openRequest("req-select")
        let send = requireElement("send-button")
        XCTAssertFalse(send.isEnabled)
        XCTAssertFalse(element("answer-text-0").exists, "a select-only question has no free text")
        requireElement("option-0-0").tap()
        XCTAssertTrue(send.isEnabled)
        shot("13-select-only")
        send.tap()
        XCTAssertTrue(gone("inbox-row-req-select", timeout: 15))
    }

    func testWhatsNextQuestionOffersTodoOptions() {
        launch()
        openRequest("req-next")
        let send = requireElement("send-button")
        requireElement("option-0-1").tap()
        XCTAssertTrue(send.isEnabled)
        shot("14-whats-next")
        send.tap()
        XCTAssertTrue(gone("inbox-row-req-next", timeout: 15))
    }

    func testRequestCanBeCancelledFromTheMenu() {
        launch()
        openRequest("req-select")
        requireElement("request-menu").tap()
        let cancel = app.buttons.matching(NSPredicate(format: "label == %@", language == "en" ? "Cancel request" : "取消请求")).firstMatch
        XCTAssertTrue(cancel.waitForExistence(timeout: 5))
        cancel.tap()
        shot("15-cancel-dialog")
        let confirm = app.buttons.matching(NSPredicate(format: "label == %@", language == "en" ? "Cancel request" : "取消请求")).element(boundBy: 0)
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        confirm.tap()
        XCTAssertTrue(gone("inbox-row-req-select", timeout: 15))
    }

    // MARK: confirmations

    func testAllowingAConfirmationNeedsAChoiceFirst() {
        launch()
        openRequest("req-permission")
        let submit = requireElement("submit-button")
        XCTAssertFalse(submit.isEnabled)
        shot("20-confirm")
        requireElement("choice-0").tap()
        XCTAssertTrue(submit.isEnabled)
        XCTAssertFalse(element("confirm-input").exists, "the reason only applies to a refusal")
        submit.tap()
        XCTAssertTrue(gone("inbox-row-req-permission", timeout: 15))
    }

    func testRefusingAConfirmationShowsTheOptionalReason() {
        launch()
        openRequest("req-permission")
        requireElement("choice-2").tap()
        _ = requireElement("confirm-input")
        shot("21-confirm-deny")
        type("Run the tests first.", into: "confirm-input")
        requireElement("submit-button").tap()
        XCTAssertTrue(gone("inbox-row-req-permission", timeout: 15))
    }

    func testContinuingWorkRequiresAnExplanation() {
        launch()
        openRequest("req-stop")
        let submit = requireElement("submit-button")
        requireElement("choice-1").tap()
        XCTAssertFalse(submit.isEnabled, "the explanation is required for this choice")
        type("Also run the Windows build.", into: "confirm-input")
        XCTAssertTrue(submit.isEnabled)
        shot("22-confirm-required-input")
        submit.tap()
        XCTAssertTrue(gone("inbox-row-req-stop", timeout: 15))
    }

    // MARK: history

    func testHistoryListsFinishedRequestsAndOpensThem() {
        launch(extra: ["-AHTab", "history"])
        let row = requireElement("history-row-req-old-1")
        _ = requireElement("history-row-req-old-2")
        shot("30-history")
        row.tap()
        XCTAssertTrue(app.navigationBars.firstMatch.waitForExistence(timeout: 5))
        shot("31-history-detail")
    }

    func testAnsweredRequestsAppearInTheHistory() {
        launch()
        openRequest("req-select")
        requireElement("option-0-0").tap()
        requireElement("send-button").tap()
        XCTAssertTrue(gone("inbox-row-req-select", timeout: 15))
        tab(2)
        _ = requireElement("history-row-req-select")
    }

    // MARK: Macs and pairing

    func testMacsTabListsTheMacsAndOpensOne() {
        launch(extra: ["-AHTab", "macs"])
        let home = requireElement("mac-row-\(homeMac)")
        _ = requireElement("mac-row-\(officeMac)")
        shot("40-macs")
        home.tap()
        XCTAssertTrue(app.navigationBars.firstMatch.waitForExistence(timeout: 5))
        shot("41-mac-detail")
    }

    func testFirstLaunchOffersPairingAndWalksThroughIt() {
        launch("empty", latency: 2)
        let start = requireElement("pair-a-mac")
        shot("50-onboarding")
        start.tap()
        let code = requireElement("pairing-code")
        code.tap()
        code.typeText("ahpair1.example")
        shot("51-pairing-code")
        requireElement("pair-button").tap()
        _ = requireElement("pairing-sas", timeout: 15)
        shot("52-pairing-sas")
    }

    func testAMalformedPairingCodeIsRefused() {
        launch("empty")
        requireElement("pair-a-mac").tap()
        let code = requireElement("pairing-code")
        code.tap()
        code.typeText("not-a-code")
        requireElement("pair-button").tap()
        _ = requireElement("pairing-retry", timeout: 15)
        shot("53-pairing-invalid")
    }

    // MARK: connection states

    func testAnEmptyInboxSaysSo() {
        launch()
        // Answer everything, one after the other, until nothing is left.
        for id in ["req-permission", "req-stop"] {
            openRequest(id)
            requireElement("choice-0").tap()
            requireElement("submit-button").tap()
            XCTAssertTrue(gone("inbox-row-\(id)", timeout: 15))
        }
        for id in ["req-select", "req-next"] {
            openRequest(id)
            tapOption("option-0-0")
            requireElement("send-button").tap()
            XCTAssertTrue(gone("inbox-row-\(id)", timeout: 15))
        }
        openRequest("req-migration")
        tapOption("option-0-0")
        tapOption("option-1-1")
        type("ok", into: "answer-text-2")
        requireElement("send-button").tap()
        XCTAssertTrue(gone("inbox-row-req-migration", timeout: 15))
        shot("60-inbox-empty")
    }

    func testAnOfflineMacKeepsRequestsReadableButNotAnswerable() {
        launch("offline")
        _ = requireElement("inbox-row-req-select")
        shot("61-inbox-offline")
        openRequest("req-select")
        let send = requireElement("send-button")
        requireElement("option-0-0").tap()
        XCTAssertFalse(send.isEnabled, "nothing can be sent while the Mac is offline")
        shot("62-answer-offline")
    }

    func testTheInboxWarnsBeforeTheFreeDataAllowanceRunsOut() {
        launch("quota")
        _ = requireElement("inbox-row-req-select")
        let notice = requireElement("notice-traffic")
        XCTAssertTrue(notice.label.contains(localized("close to the broker's free allowance", "快用完")), notice.label)
        shot("63-inbox-quota")
    }

    /// How long the mocked first sync takes in the tests below. The title only says "Updating…" while
    /// it lasts, and every query of the UI test costs about a second after a launch that takes several
    /// more: half a minute was gone before a test looked once the machine was busy with a build, so the
    /// waits for the end of the sync outlast it by a good margin.
    private static let syncDelay = "20"
    private static let syncWait: TimeInterval = 35

    func testTheInboxWaitsForTheMacsInsteadOfClaimingToBeEmpty() {
        launch("syncing", extra: ["-AHSyncDelay", Self.syncDelay])
        _ = requireElement("loading-placeholder")
        shot("64-inbox-loading")
        XCTAssertFalse(app.staticTexts[localized("All caught up", "已全部处理")].exists, "nothing is known yet, so nothing is 'caught up'")
        XCTAssertFalse(element("inbox-row-req-select").exists)
        // The title says so, but only after a short delay.
        _ = requireElement("sync-status")
        // The rows replace the placeholders once the Macs answered, and the title is itself again.
        _ = requireElement("inbox-row-req-select", timeout: Self.syncWait)
        XCTAssertTrue(gone("loading-placeholder"))
        XCTAssertTrue(gone("sync-status"))
        shot("65-inbox-synced")
    }

    func testRequestsKeptFromTheLastRunShowWhileTheTitleSaysUpdating() {
        launch("syncing", extra: ["-AHCached", "YES", "-AHSyncDelay", Self.syncDelay])
        _ = requireElement("inbox-row-req-select")
        XCTAssertFalse(element("loading-placeholder").exists, "there is something to show, so no placeholders")
        _ = requireElement("sync-status")
        shot("66-inbox-updating")
        XCTAssertTrue(gone("sync-status", timeout: Self.syncWait))
        XCTAssertTrue(element("inbox-row-req-select").exists, "the rows stay as they are")
    }

    func testAnUnreachableBrokerIsQuietAtFirstAndSaysSoLater() {
        launch("offline")
        // Asked first: the notice only follows after about ten seconds of reconnecting.
        XCTAssertFalse(element("notice-broker").exists, "nothing pops in while it is only reconnecting")
        _ = requireElement("inbox-row-req-select")
        _ = requireElement("sync-status")
        shot("67-inbox-reconnecting")
        let notice = requireElement("notice-broker", timeout: 25)
        XCTAssertTrue(notice.exists)
        shot("68-inbox-broker-notice")
    }

    func testTheAgentsTabWaitsForTheMacsToo() {
        launch("syncing", extra: ["-AHTab", "agents", "-AHSyncDelay", Self.syncDelay])
        _ = requireElement("loading-placeholder")
        shot("85-agents-loading")
        XCTAssertFalse(app.staticTexts[localized("No Agent sessions", "没有 Agent 会话")].exists)
        XCTAssertFalse(app.staticTexts[localized("Your Macs are not reachable", "连不上你的 Mac")].exists, "still connecting is not unreachable")
        _ = requireElement("agent-row-\(workingAgent)", timeout: Self.syncWait)
        XCTAssertTrue(gone("loading-placeholder"))
    }

    // MARK: a tapped notification

    private var unavailableTitle: String { localized("Request unavailable", "请求不可用") }

    func testATappedNotificationShowsItsTextWhileTheRequestIsOnItsWay() {
        launch("syncing", extra: ["-AHSyncDelay", Self.syncDelay, "-AHNotify", "req-migration"])
        // The page opens at once from what the notification said; the request itself is not here yet.
        _ = requireElement("request-placeholder-hint")
        XCTAssertTrue(app.staticTexts["Claude Code · HumanInLoop"].exists)
        XCTAssertTrue(app.staticTexts["Which database should the migration target?"].exists)
        XCTAssertFalse(app.staticTexts[unavailableTitle].exists, "it is on its way, not unavailable")
        shot("90-request-on-its-way")
        // The form takes its place once the Macs have answered.
        _ = requireElement("next-button", timeout: Self.syncWait)
        XCTAssertTrue(gone("request-placeholder"))
        shot("91-request-arrived")
    }

    func testARequestTheMacDoesNotHaveIsUnavailableOnlyOnceTheMacHasAnswered() {
        launch("syncing", extra: ["-AHSyncDelay", Self.syncDelay, "-AHNotify", "req-answered-elsewhere"])
        _ = requireElement("request-placeholder")
        XCTAssertFalse(app.staticTexts[unavailableTitle].exists, "the Macs have not answered yet")
        XCTAssertTrue(app.staticTexts[unavailableTitle].waitForExistence(timeout: Self.syncWait))
        XCTAssertTrue(gone("request-placeholder"))
        shot("92-request-gone")
    }

    func testARequestOfAMacOutOfReachSaysSoInsteadOfBeingUnavailable() {
        launch("offline", extra: ["-AHNotify", "req-late"])
        _ = requireElement("request-placeholder-hint")
        XCTAssertTrue(app.staticTexts[localized("Your Mac is not reachable", "连不上你的 Mac")].exists)
        XCTAssertFalse(app.staticTexts[unavailableTitle].exists)
        shot("93-request-unreachable")
    }

    // MARK: moving on after sending

    func testAfterSendingThePageShowsAReceiptThenTheNextRequestSlidesIn() {
        launch(autoAdvance: true, extra: ["-AHAdvanceHold", "2", "-AHAdvanceSettle", "1.2"])
        openRequest("req-select")
        requireElement("option-0-0").tap()
        requireElement("send-button").tap()
        _ = requireElement("sent-receipt", timeout: 10)
        shot("18-sent-receipt")
        XCTAssertFalse(element("send-button").isHittable, "the receipt takes the form's place")
        // The next request is pushed over the sent one, with a pill saying how many are left.
        _ = requireElement("sent-pill", timeout: 10)
        shot("19-sent-pill")
        _ = requireElement("submit-button", timeout: 10)
        XCTAssertTrue(gone("sent-receipt", timeout: 10))
        // Back leads to the list, not to the page that was sent.
        app.navigationBars.buttons.firstMatch.tap()
        _ = requireElement("inbox-row-req-migration")
        XCTAssertFalse(element("inbox-row-req-select").exists)
    }

    func testWithoutAutoAdvanceTheReceiptLeadsBackToTheList() {
        launch(extra: ["-AHAdvanceHold", "1.5"])
        openRequest("req-select")
        requireElement("option-0-0").tap()
        requireElement("send-button").tap()
        _ = requireElement("sent-receipt", timeout: 10)
        XCTAssertTrue(gone("sent-receipt", timeout: 10))
        _ = requireElement("inbox-row-req-migration")
        XCTAssertFalse(element("sent-pill").exists, "there is no next request to point at")
    }

    // MARK: agents

    func testAgentsTabListsSessionsByStateAndFilters() {
        launch(extra: ["-AHTab", "agents"])
        for key in [workingAgent, waitingAgent, idleAgent, endedAgent, officeAgent, unsupportedAgent] {
            _ = requireElement("agent-row-\(key)")
        }
        _ = requireElement("new-task")
        XCTAssertFalse(element("filter-note").exists)
        shot("70-agents")

        chooseFilter(localized("Following", "已关注"))
        XCTAssertTrue(gone("agent-row-\(waitingAgent)"))
        XCTAssertTrue(element("agent-row-\(workingAgent)").exists)
        XCTAssertTrue(element("agent-row-\(officeAgent)").exists)
        _ = requireElement("filter-note")
        shot("71-agents-following")

        chooseFilter("Office iMac")
        XCTAssertTrue(gone("agent-row-\(workingAgent)"))
        XCTAssertTrue(element("agent-row-\(officeAgent)").exists)
        XCTAssertTrue(element("agent-row-\(unsupportedAgent)").exists)

        requireElement("filter-clear").tap()
        _ = requireElement("agent-row-\(waitingAgent)")
    }

    func testALongCurrentToolDoesNotWidenTheAgentRow() {
        launch(extra: ["-AHTab", "agents"])
        // The office Agent is running "Shell: cargo test --manifest-path …", far wider than a phone.
        let row = requireElement("agent-row-\(officeAgent)")
        let screen = app.windows.firstMatch.frame
        XCTAssertGreaterThanOrEqual(row.frame.minX, 0)
        XCTAssertLessThanOrEqual(row.frame.maxX, screen.maxX + 0.5, "the row grew past the screen: \(row.frame)")
        shot("84-agent-long-tool")
    }

    func testAWorkingAgentShowsALiveViewAndCanBeUnfollowed() {
        launch(extra: ["-AHTab", "agents"])
        requireElement("agent-row-\(workingAgent)").tap()
        _ = requireElement("watch-card")
        _ = requireElement("watch-live")
        shot("72-agent-working")

        XCTAssertTrue(waitForLabel("follow-button", localized("Following", "已关注")))
        requireElement("follow-button").tap()
        XCTAssertTrue(waitForLabel("follow-button", localized("Follow", "关注")), "the Mac took the follow back")
        requireElement("follow-button").tap()
        XCTAssertTrue(waitForLabel("follow-button", localized("Following", "已关注")))
    }

    func testAMessageToAWorkingAgentIsQueuedAndCanBeWithdrawn() {
        launch(extra: ["-AHTab", "agents", "-AHOpenAgent", workingAgent, "-AHHoldInterjects", "YES"])
        let field = requireElement("interject-field")
        let send = requireElement("interject-send")
        XCTAssertFalse(send.isEnabled, "nothing to send yet")
        field.tap()
        field.typeText("Also run the Windows build.")
        XCTAssertTrue(send.isEnabled)
        shot("73-interject-typed")
        send.tap()
        _ = requireElement("interject-pending")
        XCTAssertTrue(send.waitForExistence(timeout: 2))
        XCTAssertFalse(send.isEnabled, "the form is empty again")
        shot("74-interject-pending")
        requireElement("interject-withdraw").tap()
        XCTAssertTrue(gone("interject-pending"))
    }

    func testEditingQueuedMessagesPutsTheirTextBack() {
        launch(extra: ["-AHTab", "agents", "-AHOpenAgent", workingAgent, "-AHHoldInterjects", "YES"])
        let field = requireElement("interject-field")
        field.tap()
        field.typeText("Check the Linux build too.")
        requireElement("interject-send").tap()
        requireElement("interject-edit").tap()
        XCTAssertTrue(gone("interject-pending"))
        XCTAssertEqual(requireElement("interject-field").value as? String, "Check the Linux build too.")
    }

    func testTheAgentPickingUpAMessageIsAnnounced() {
        launch(extra: ["-AHTab", "agents", "-AHOpenAgent", workingAgent])
        let field = requireElement("interject-field")
        field.tap()
        field.typeText("Please keep going.")
        requireElement("interject-send").tap()
        XCTAssertTrue(waitForLabel("interject-notice", localized("The Agent read your message.", "Agent 已读取你的消息。"), timeout: 15))
        shot("75-interject-read")
        XCTAssertTrue(gone("interject-pending"))
    }

    func testAnAgentThatCannotReceiveMessagesSaysWhy() {
        launch(extra: ["-AHTab", "agents", "-AHOpenAgent", unsupportedAgent])
        _ = requireElement("interject-unavailable")
        XCTAssertFalse(element("interject-field").exists)
        shot("76-agent-unsupported")
    }

    func testAnAgentWaitingForAnAnswerLinksToItsQuestion() {
        launch(extra: ["-AHTab", "agents", "-AHOpenAgent", waitingAgent])
        shot("77-agent-waiting")
        requireElement("answer-waiting").tap()
        _ = requireElement("answered-count")
    }

    func testAFollowedAgentFinishingShowsABannerThatOpensIt() {
        launch(extra: ["-AHTab", "agents", "-AHFollowEvent", "done"])
        let banner = requireElement("banner", timeout: 15)
        shot("78-follow-banner")
        XCTAssertTrue(banner.label.contains(localized("Agent finished", "Agent 已完成")))
        banner.tap()
        _ = requireElement("watch-card")
    }

    func testCreatingATaskWalksThroughTheFormAndOpensTheNewAgent() {
        launch(extra: ["-AHTab", "agents"])
        requireElement("new-task").tap()
        let start = requireElement("task-start")
        XCTAssertFalse(start.isEnabled)
        shot("79-task-form")
        // The Mac asks every time: nothing is preselected, so the choice comes first.
        tapOption("task-permission-default")
        XCTAssertFalse(start.isEnabled, "still no text")
        type("Add a dark mode toggle to the settings page.", into: "task-text")
        XCTAssertTrue(start.isEnabled)
        shot("80-task-filled")
        start.tap()
        _ = requireElement("task-progress")
        shot("81-task-progress")
        requireElement("task-open-agent", timeout: 30).tap()
        _ = requireElement("watch-card")
    }

    func testLeavingAFilledTaskFormAsksBeforeDiscarding() {
        launch(extra: ["-AHTab", "agents", "-AHNewTask", "YES"])
        type("A task I may not send.", into: "task-text")
        requireElement("task-cancel").tap()
        let discard = app.buttons.matching(NSPredicate(format: "label == %@", localized("Discard", "放弃"))).firstMatch
        XCTAssertTrue(discard.waitForExistence(timeout: 5))
        shot("82-task-discard")
        discard.tap()
        XCTAssertTrue(gone("task-start"))
    }

    func testAMacCanBeToldToSendQuestionsToThisPhone() {
        launch(extra: ["-AHTab", "macs"])
        requireElement("mac-row-\(homeMac)").tap()
        reveal("take-slot")
        shot("83-take-slot")
        requireElement("take-slot").tap()
        XCTAssertTrue(gone("take-slot", timeout: 15))
    }
}
