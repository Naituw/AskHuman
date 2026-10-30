import XCTest

/// The model and effort of the new task form (`docs/specs/ios-task-model.md`) through the real UI over the
/// mock backend, in English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`). The demo Mac offers Claude (the same
/// five efforts for every model), Cursor (twenty models, no efforts), Codex (efforts and a default per model)
/// and Pi (some models think, some do not); Grok is not ready.
@MainActor
final class AskHumanModelUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    private let workingAgent = "claude:cc-flaky"
    private let waitingAgent = "cursor:cur-migrate"
    private let endedAgent = "cursor:cur-readme"

    override func setUp() async throws {
        continueAfterFailure = false
    }

    // MARK: helpers

    @discardableResult
    private func launch(latency: Double = 0.05, extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", "demo", "-AHLatency", "\(latency)", "-AHAutoAdvance", "NO",
            "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ] + extra
        app.launch()
        self.app = app
        return app
    }

    /// The new task form, open on the demo Mac, once its choices are there.
    private func openForm(extra: [String] = []) {
        launch(extra: ["-AHTab", "agents", "-AHNewTask", "YES"] + extra)
        _ = requireElement("task-agent-claude")
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func requireElement(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let found = element(id)
        XCTAssertTrue(found.waitForExistence(timeout: timeout), "\(id) did not appear", file: file, line: line)
        return found
    }

    private func gone(_ id: String, timeout: TimeInterval = 10) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element(id))
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    /// Waits until the accessibility label of the element contains `text`.
    private func waitForLabel(_ id: String, containing text: String, timeout: TimeInterval = 10) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label CONTAINS %@", text), object: element(id))
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
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
        attachment.name = "\(language)-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// Scrolls the form until the element exists (a lazy form only creates the rows near the visible area).
    private func reveal(_ id: String) {
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
            if frame.minY <= 120 { app.swipeDown(velocity: .slow) } else { app.swipeUp(velocity: .slow) }
        }
    }

    private func type(_ text: String, into id: String) {
        reveal(id)
        let field = requireElement(id)
        field.tap()
        field.typeText(text)
    }

    /// Picks a model on its page and waits for the form to show it.
    private func pickModel(_ id: String, shownAs name: String) {
        requireElement("task-model").tap()
        _ = requireElement("model-picker")
        requireElement("model-option-\(id)").tap()
        XCTAssertTrue(waitForLabel("task-model", containing: name), "the form does not show \(name)")
    }

    /// Chooses an effort in the menu of its row, and waits for the row to show it.
    private func pickEffort(_ title: String) {
        requireElement("task-effort").tap()
        let entry = app.buttons.matching(NSPredicate(format: "label == %@", title)).firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 5), "the effort menu has no entry \(title)")
        entry.tap()
        XCTAssertTrue(waitForLabel("task-effort", containing: title), "the effort row does not show \(title)")
    }

    private func fillAndStart(_ text: String = "Add a dark mode toggle to the settings page.") {
        reveal("task-permission-default")
        requireElement("task-permission-default").tap()
        type(text, into: "task-text")
        requireElement("task-start").tap()
        _ = requireElement("task-progress")
    }

    // MARK: the form

    func testTheFormStartsFromDefaultAndOffersTheModelAndTheEffort() {
        openForm()
        let model = requireElement("task-model")
        XCTAssertTrue(model.label.contains(localized("Default", "默认")), model.label)
        XCTAssertTrue(
            model.label.contains("opus"), "the line under it says what Default stands for: \(model.label)"
        )
        let effort = requireElement("task-effort")
        XCTAssertTrue(effort.label.contains(localized("Default", "默认")), effort.label)
        shot("110-model-form-default")
    }

    func testPickingAModelAndAnEffortShowsThemOnTheProgressPage() {
        openForm()
        pickModel("sonnet", shownAs: "Sonnet")
        shot("111-model-picked")
        pickEffort(localized("High", "高"))
        shot("112-effort-picked")
        fillAndStart()
        let line = requireElement("task-model-line")
        XCTAssertEqual(line.label, localized("Model: Sonnet · Effort: High", "模型：Sonnet · 强度：高"))
        shot("113-model-progress")
    }

    func testAnEffortAloneIsShownWithDefaultAsTheModel() {
        openForm()
        pickEffort(localized("Max", "最高"))
        fillAndStart()
        XCTAssertEqual(
            requireElement("task-model-line").label, localized("Model: Default · Effort: Max", "模型：默认 · 强度：最高")
        )
    }

    func testNothingPickedLeavesNoLineOnTheProgressPage() {
        openForm()
        fillAndStart()
        XCTAssertTrue(gone("task-model-line", timeout: 2), "nothing was picked, so nothing is claimed")
    }

    func testTheModelPageListsDefaultFirstAndMarksTheChoice() {
        openForm()
        requireElement("task-model").tap()
        _ = requireElement("model-picker")
        let first = requireElement("model-option-default")
        XCTAssertTrue(first.isSelected, "nothing was picked yet")
        _ = requireElement("model-option-opus")
        XCTAssertFalse(app.searchFields.firstMatch.exists, "three models need no search")
        shot("114-model-page")
        requireElement("model-option-haiku").tap()
        XCTAssertTrue(waitForLabel("task-model", containing: "Haiku"))
        requireElement("task-model").tap()
        XCTAssertTrue(requireElement("model-option-haiku").isSelected)
        XCTAssertFalse(requireElement("model-option-default").isSelected)
    }

    func testALongModelListHasASearchFieldThatFiltersByNameAndId() {
        openForm()
        requireElement("task-agent-cursor").tap()
        requireElement("task-model").tap()
        _ = requireElement("model-picker")
        let search = app.searchFields.firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 5), "twenty models need a search field")
        shot("115-model-search")

        search.tap()
        search.typeText("kimi")
        _ = requireElement("model-option-kimi-k2")
        XCTAssertTrue(gone("model-option-gpt-5", timeout: 5), "the others are filtered out")
        _ = requireElement("model-option-default")

        // The id matches too, not only the name: "max-high" is in no display name.
        search.clearAndType("max-high")
        _ = requireElement("model-option-gpt-5.1-codex-max-high")
        XCTAssertTrue(gone("model-option-kimi-k2", timeout: 5))

        search.clearAndType("zzzz")
        XCTAssertTrue(gone("model-option-gpt-5.1-codex-max-high", timeout: 5))
        shot("116-model-search-empty")

        search.clearAndType("kimi")
        requireElement("model-option-kimi-k2").tap()
        XCTAssertTrue(waitForLabel("task-model", containing: "Kimi K2"))
        XCTAssertTrue(gone("task-effort", timeout: 3), "Cursor has no effort row")
    }

    func testSwitchingTheAgentStartsFromDefaultAgain() {
        openForm()
        pickModel("sonnet", shownAs: "Sonnet")
        pickEffort(localized("High", "高"))
        requireElement("task-agent-cursor").tap()
        XCTAssertTrue(waitForLabel("task-model", containing: localized("Default", "默认")))
        requireElement("task-agent-claude").tap()
        XCTAssertTrue(waitForLabel("task-model", containing: localized("Default", "默认")), "coming back does not restore the pick")
        XCTAssertFalse(requireElement("task-model").label.contains("Sonnet"))
        XCTAssertFalse(requireElement("task-effort").label.contains(localized("High", "高")))
        shot("117-model-reset")
    }

    func testStartingATaskRemembersTheModelAndEffortForTheNextForm() {
        openForm()
        pickModel("sonnet", shownAs: "Sonnet")
        pickEffort(localized("High", "高"))
        fillAndStart()
        requireElement("task-close").tap()
        requireElement("new-task").tap()
        XCTAssertTrue(waitForLabel("task-model", containing: "Sonnet"))
        XCTAssertTrue(waitForLabel("task-effort", containing: localized("High", "高")))
        requireElement("task-agent-cursor").tap()
        XCTAssertTrue(waitForLabel("task-model", containing: localized("Default", "默认")))
        requireElement("task-agent-claude").tap()
        XCTAssertTrue(waitForLabel("task-model", containing: "Sonnet"))
        XCTAssertTrue(waitForLabel("task-effort", containing: localized("High", "高")))
    }

    func testTheEffortRowFollowsTheModelThatIsPicked() {
        openForm()
        requireElement("task-agent-pi").tap()
        _ = requireElement("task-effort")
        pickModel("ollama/llama3.2", shownAs: "llama3.2")
        XCTAssertTrue(gone("task-effort", timeout: 5), "that model does not think: no effort row")
        shot("118-pi-no-effort")
        pickModel("openai/gpt-5", shownAs: "gpt-5")
        _ = requireElement("task-effort")
        pickModel("default", shownAs: localized("Default", "默认"))
        _ = requireElement("task-effort")
    }

    func testTheEffortRowSaysWhatDefaultStandsForWhenTheMacKnows() {
        openForm()
        requireElement("task-agent-codex").tap()
        // Default stands for a model the Mac configured with `high`.
        XCTAssertTrue(waitForLabel("task-effort", containing: localized("Default (High)", "默认（高）")), element("task-effort").label)
        // A model of its own runs with its own default.
        pickModel("gpt-5.1-codex-mini", shownAs: "Mini")
        XCTAssertTrue(waitForLabel("task-effort", containing: localized("Default (Medium)", "默认（中）")), element("task-effort").label)
        shot("119-codex-default-effort")
    }

    func testAnOlderMacShowsNeitherRowAndTheTaskStillStarts() {
        openForm(extra: ["-AHNoModels", "YES"])
        XCTAssertTrue(gone("task-model", timeout: 3))
        XCTAssertTrue(gone("task-effort", timeout: 3))
        shot("120-no-models")
        fillAndStart()
        XCTAssertTrue(gone("task-model-line", timeout: 2))
        _ = requireElement("task-open-agent", timeout: 30)
    }

    func testAModelTheMacRefusesIsExplained() {
        openForm(extra: ["-AHFailTask", "modelUnavailable"])
        pickModel("sonnet", shownAs: "Sonnet")
        fillAndStart()
        let reason = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", localized("no longer offers that model", "已不再提供这个模型"))
        ).firstMatch
        XCTAssertTrue(reason.waitForExistence(timeout: 15), "the progress page does not say why")
        XCTAssertEqual(requireElement("task-model-line").label, localized("Model: Sonnet", "模型：Sonnet"))
        shot("121-model-refused")
    }

    func testAnEffortTheMacRefusesIsExplained() {
        openForm(extra: ["-AHFailTask", "effortUnavailable"])
        pickEffort(localized("Low", "低"))
        fillAndStart()
        let reason = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", localized("not available for the chosen model", "所选模型不支持这个强度"))
        ).firstMatch
        XCTAssertTrue(reason.waitForExistence(timeout: 15), "the progress page does not say why")
    }

    // MARK: the Agent's page

    func testTheAgentPageShowsTheModelAndEffortItRunsWith() {
        launch(extra: ["-AHTab", "agents", "-AHOpenAgent", workingAgent])
        let line = requireElement("agent-model")
        XCTAssertEqual(line.label, localized("Model: claude-opus-4-1 · Effort: High", "模型：claude-opus-4-1 · 强度：高"))
        shot("122-agent-model")
    }

    func testAnAgentWithoutAnEffortShowsTheModelAlone() {
        launch(extra: ["-AHTab", "agents", "-AHOpenAgent", waitingAgent])
        XCTAssertEqual(requireElement("agent-model").label, localized("Model: composer-1", "模型：composer-1"))
    }

    func testAnAgentTheMacKnowsNothingAboutShowsNoModelLine() {
        launch(extra: ["-AHTab", "agents", "-AHOpenAgent", endedAgent])
        _ = requireElement("watch-card")
        XCTAssertTrue(gone("agent-model", timeout: 3))
    }
}

private extension XCUIElement {
    /// Replaces the text of a search field.
    func clearAndType(_ text: String) {
        tap()
        if let current = value as? String, !current.isEmpty, current != "Search" {
            typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count))
        }
        typeText(text)
    }
}
