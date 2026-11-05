import XCTest

/// The project's todos in the answer form (`docs/specs/special-replies.md` D28/D29) through the real UI over the
/// mock Macs, in English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`). The `todoPick` scenario gives the Home
/// MacBook Pro's requests a queue of seven todos and Macs that take them with an answer; the plain demo has Macs
/// from before that.
@MainActor
final class AskHumanTodoPickUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    override func setUp() async throws {
        continueAfterFailure = false
    }

    override func tearDown() async throws {
        // What the screen looked like when a test failed.
        if let run = testRun, run.totalFailureCount > 0, app != nil { shot("failed-" + name.filter { $0.isLetter }) }
    }

    // MARK: helpers

    @discardableResult
    private func launch(_ scenario: String = "todoPick") -> XCUIApplication {
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", scenario, "-AHLatency", "0.05", "-AHAutoAdvance", "NO", "-AHWebMarkdown", "NO",
            "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
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

    private func openRequest(_ id: String) {
        requireElement("inbox-row-\(id)").tap()
    }

    /// Scrolls until the element exists (a lazy form only makes the rows near the visible area), then until it sits
    /// clear of the bars. `false` when it never showed up.
    @discardableResult
    private func reveal(_ id: String, maxSwipes: Int = 10) -> Bool {
        var swipes = 0
        while !element(id).exists, swipes < maxSwipes {
            app.swipeUp(velocity: .slow)
            swipes += 1
        }
        guard element(id).exists else { return false }
        let screen = app.windows.firstMatch.frame
        for _ in 0..<8 {
            let frame = element(id).frame
            if frame.minY > 120, frame.maxY < screen.height - 220 { break }
            if frame.minY <= 120 { app.swipeDown(velocity: .slow) } else { app.swipeUp(velocity: .slow) }
        }
        return true
    }

    private func answered(_ n: Int, of total: Int) -> String {
        localized("Answered \(n)/\(total)", "已回答 \(n)/\(total)")
    }

    private func selected(_ n: Int) -> String { localized("\(n) selected", "已选 \(n)") }

    // MARK: an ordinary question

    func testTheTodosCardIsFoldedAndOpensToRowsThatCanBePicked() {
        launch()
        openRequest("req-migration")
        XCTAssertTrue(reveal("todo-pick-header"), "the card under the last question")
        XCTAssertFalse(element("todo-row-0").exists, "folded: a single line")
        XCTAssertFalse(element("todo-pick-count").exists, "nothing is picked yet")
        shot("todo-pick-1-folded")

        element("todo-pick-header").tap()
        requireElement("todo-row-0")
        XCTAssertTrue(element("todo-row-6").exists, "the whole queue of seven")
        XCTAssertTrue(requireElement("todo-pick-footnote").exists)

        reveal("todo-row-1")
        element("todo-row-1").tap()
        reveal("todo-row-3")
        element("todo-row-3").tap()
        XCTAssertTrue(element("todo-row-1").isSelected)
        XCTAssertTrue(element("todo-row-3").isSelected)
        XCTAssertFalse(element("todo-row-0").isSelected, "several can be on together")
        XCTAssertEqual(requireElement("todo-pick-count").label, selected(2))
        shot("todo-pick-2-picked")

        element("todo-row-1").tap()
        XCTAssertFalse(element("todo-row-1").isSelected, "tapping again lets go")
        XCTAssertEqual(requireElement("todo-pick-count").label, selected(1))
    }

    func testATodoAloneIsAnAnswerAndLeavesTheQueueOnceSent() {
        launch()
        openRequest("req-migration")
        XCTAssertTrue(reveal("todo-pick-header"))
        element("todo-pick-header").tap()
        reveal("todo-row-0")
        element("todo-row-0").tap()
        reveal("todo-row-3")
        element("todo-row-3").tap()
        let send = requireElement("send-button")
        XCTAssertTrue(send.isEnabled)
        send.tap()
        XCTAssertTrue(gone("inbox-row-req-migration", timeout: 15))

        // What's next of the same project: the two picked ones are not in the queue any more.
        openRequest("req-next")
        requireElement("todo-row-0")
        XCTAssertTrue(element("todo-row-2").exists)
        XCTAssertFalse(element("todo-row-5").exists, "seven less two")
        XCTAssertFalse(element("todo-show-all").exists, "five fit without \"Show all\"")
        shot("todo-pick-3-after-send")
    }

    func testASelectOnlyRequestHasNoTodosCard() {
        launch()
        openRequest("req-select")
        requireElement("option-0-0")
        XCTAssertFalse(reveal("todo-pick-header", maxSwipes: 2))
    }

    // MARK: What's next

    func testWhatsNextListsTheFirstFiveAndShowAllOpensTheRest() {
        launch()
        openRequest("req-next")
        requireElement("todo-row-0")
        XCTAssertTrue(element("todo-row-4").exists)
        XCTAssertFalse(element("todo-row-5").exists, "the first five")
        XCTAssertFalse(element("option-0-0").exists, "the queue takes the place of the todos the question carried")
        shot("todo-next-1-first-five")

        let showAll = requireElement("todo-show-all")
        XCTAssertEqual(showAll.label, localized("Show all 7 todos", "显示全部 7 条待办"))
        showAll.tap()
        reveal("todo-row-6")
        XCTAssertTrue(element("todo-row-6").exists)
        XCTAssertEqual(requireElement("todo-show-all").label, localized("Show less", "收起"))
        shot("todo-next-2-all")

        element("todo-row-6").tap()
        XCTAssertTrue(element("todo-row-6").isSelected)
        XCTAssertEqual(requireElement("answered-count").label, answered(1, of: 1))
    }

    func testWhatsNextTakesOneTodoAtATimeAndSendsIt() {
        launch()
        openRequest("req-next")
        requireElement("todo-row-0").tap()
        element("todo-row-1").tap()
        XCTAssertTrue(element("todo-row-1").isSelected)
        XCTAssertFalse(element("todo-row-0").isSelected, "the next task is one thing")
        let send = requireElement("send-button")
        XCTAssertTrue(send.isEnabled)
        send.tap()
        XCTAssertTrue(gone("inbox-row-req-next", timeout: 15))
    }

    // MARK: a Mac from before it

    func testAMacFromBeforeTheTodoPickerShowsNoCardAndKeepsWhatTheQuestionCarries() {
        launch("demo")
        openRequest("req-migration")
        XCTAssertFalse(reveal("todo-pick-header", maxSwipes: 4), "the Mac would drop the ids")
        app.terminate()

        launch("demo")
        openRequest("req-next")
        requireElement("option-0-0")
        XCTAssertFalse(element("todo-row-0").exists)
        XCTAssertFalse(element("todo-show-all").exists)
    }
}
