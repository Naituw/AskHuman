import XCTest

/// The project's todos in the answer form (`docs/specs/special-replies.md` D28/D29, `docs/specs/todo-list-actions.md`)
/// through the real UI over the mock Macs, in English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`). The `todoPick`
/// scenario gives the Home MacBook Pro's requests a queue of seven todos and Macs that take them with an answer; the plain
/// demo has Macs from before that. `req-stop-ask` is a Stop confirmation of the same project and `req-fresh` an ask of
/// a project that has no todos.
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

    /// `hold` is how long the notes over the list ("Completed … · Undo") stay: long enough for a slow simulator, unless a
    /// test is about them going.
    @discardableResult
    private func launch(_ scenario: String = "todoPick", hold: Double = 30) -> XCUIApplication {
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", scenario, "-AHLatency", "0.05", "-AHAutoAdvance", "NO", "-AHWebMarkdown", "NO",
            "-AHTodoNoteHold", "\(hold)",
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

    /// The inbox is a lazy list: a row below the fold is only made once it is scrolled to.
    private func openRequest(_ id: String) {
        var scrolls = 0
        while !element("inbox-row-\(id)").waitForExistence(timeout: scrolls == 0 ? 20 : 2), scrolls < 4 {
            app.swipeUp()
            scrolls += 1
        }
        requireElement("inbox-row-\(id)").tap()
    }

    /// A finger on a row, sideways: from `from` to `to`, as fractions of the row's width.
    private func drag(_ id: String, from: CGFloat, to: CGFloat) {
        let row = element(id)
        let start = row.coordinate(withNormalizedOffset: CGVector(dx: from, dy: 0.5))
        let end = row.coordinate(withNormalizedOffset: CGVector(dx: to, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: end)
    }

    /// Opens the card of an ordinary ask (it is folded at first), with `row` in view.
    private func openTheCard(_ row: String = "todo-row-0") {
        XCTAssertTrue(reveal("todo-pick-header"), "the card under the last question")
        element("todo-pick-header").tap()
        requireElement(row)
    }

    private let texts = [
        "补全 Agents 列表的空态", "实现通知扩展的解密", "整理 Markdown 渲染的快照测试", "把弹窗里的待办面板同步到手机，选中后随回答一起发送",
        "设置页增加离开模式的说明文案", "排查 Windows 下托盘图标偏移", "给 CLI 增加 --escalate 参数的文档",
    ]

    /// Types into the add row's input, the way the keyboard does.
    private func type(_ text: String) {
        let field = requireElement("todo-add-field")
        field.tap()
        field.typeText(text)
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
        XCTAssertFalse(element("todo-add-row").exists, "nothing to add to: the Mac keeps its own")
    }

    // MARK: completing and deleting in place (docs/specs/todo-list-actions.md D12)

    func testSwipingATodoAllTheWayAcrossCompletesItAndTheNoteTakesItBack() {
        launch()
        openRequest("req-migration")
        openTheCard()
        reveal("todo-row-1")
        drag("todo-row-1", from: 0.08, to: 0.97)
        requireElement("todo-undo")
        XCTAssertTrue(gone("todo-row-6"), "seven are six")
        XCTAssertTrue(element("todo-row-1").label.contains(texts[2]), "the ones below came up")
        shot("todo-actions-1-completed")

        requireElement("todo-undo-button").tap()
        requireElement("todo-row-6")
        XCTAssertTrue(element("todo-row-1").label.contains(texts[1]), "back in the place it had")
        XCTAssertTrue(gone("todo-undo"))
    }

    func testASwipeThatStopsHalfWayLeavesTheButtonToPress() {
        launch()
        openRequest("req-migration")
        openTheCard()
        reveal("todo-row-0")
        drag("todo-row-0", from: 0.08, to: 0.32)
        let complete = requireElement("todo-swipe-complete-0")
        XCTAssertTrue(element("todo-row-6").exists, "a half swipe does nothing by itself")
        shot("todo-actions-2-open")
        complete.tap()
        XCTAssertTrue(gone("todo-row-6"))
        requireElement("todo-undo")
    }

    func testSwipingLeftShowsDeleteAndOnlyPressingItDeletes() {
        launch()
        openRequest("req-migration")
        openTheCard()
        reveal("todo-row-2")
        drag("todo-row-2", from: 0.95, to: 0.05)
        let delete = requireElement("todo-swipe-delete-2")
        XCTAssertTrue(element("todo-row-6").exists, "across the whole row it still only shows the button")
        shot("todo-actions-3-delete")
        delete.tap()
        XCTAssertTrue(gone("todo-row-6"))
        XCTAssertFalse(element("todo-undo").exists, "a deleted todo has no way back")
        XCTAssertTrue(element("todo-row-2").label.contains(texts[3]))
    }

    func testATapOnARowThatIsOpenOnlyClosesIt() {
        launch()
        openRequest("req-migration")
        openTheCard()
        reveal("todo-row-1")
        drag("todo-row-1", from: 0.95, to: 0.55)
        requireElement("todo-swipe-delete-1")
        element("todo-row-1").tap()
        XCTAssertTrue(gone("todo-swipe-delete-1"))
        XCTAssertFalse(element("todo-row-1").isSelected, "the tap closed the row; it did not pick the todo")
    }

    func testAnOpenRowClosesWhenAnotherOneIsSwiped() {
        launch()
        openRequest("req-migration")
        openTheCard()
        reveal("todo-row-1")
        drag("todo-row-0", from: 0.95, to: 0.55)
        requireElement("todo-swipe-delete-0")
        drag("todo-row-1", from: 0.95, to: 0.55)
        requireElement("todo-swipe-delete-1")
        XCTAssertTrue(gone("todo-swipe-delete-0"), "one row is open at a time")
    }

    func testARowThatIsSwipedOpenKeepsItsHeightAndTheRowsUnderIt() {
        launch()
        openRequest("req-migration")
        openTheCard()
        reveal("todo-row-2")
        let before = element("todo-row-2").frame.minY
        let height = element("todo-row-1").frame.height
        drag("todo-row-1", from: 0.95, to: 0.55)
        requireElement("todo-swipe-delete-1")
        XCTAssertEqual(element("todo-row-2").frame.minY, before, accuracy: 1, "the buttons are behind the row, not part of its height")
        XCTAssertEqual(element("todo-row-1").frame.height, height, accuracy: 1)
        XCTAssertEqual(element("todo-swipe-delete-1").frame.height, height, accuracy: 1, "the button is as tall as its row")
    }

    func testAScrollThatStartsOnARowStillScrollsThePage() {
        launch()
        openRequest("req-migration")
        openTheCard()
        reveal("todo-row-5")
        let before = element("todo-row-5").frame.minY
        element("todo-row-4").swipeUp(velocity: .slow)
        var moved = element("todo-row-5").frame.minY != before
        if !moved {
            // The page was already at its end: the way back is the other direction.
            element("todo-row-4").swipeDown(velocity: .slow)
            moved = element("todo-row-5").frame.minY != before
        }
        XCTAssertTrue(moved, "the page moved")
        XCTAssertFalse(element("todo-undo").exists, "and nothing was done to the rows under the finger")
        XCTAssertTrue(element("todo-row-6").exists)
    }

    func testPressingAndHoldingARowOffersCompleteCopyAndDelete() {
        launch()
        openRequest("req-migration")
        openTheCard()
        reveal("todo-row-1")
        element("todo-row-1").press(forDuration: 1.2)
        let complete = app.buttons[localized("Complete", "完成")].firstMatch
        XCTAssertTrue(complete.waitForExistence(timeout: 5), "the menu")
        XCTAssertTrue(app.buttons[localized("Copy", "复制")].firstMatch.exists)
        let delete = app.buttons[localized("Delete", "删除")].firstMatch
        XCTAssertTrue(delete.exists)
        shot("todo-actions-4-menu")
        delete.tap()
        XCTAssertTrue(gone("todo-row-6"))
        XCTAssertFalse(element("todo-undo").exists, "the menu's Delete is the choice: nothing is asked again")
    }

    func testTheNoteOfACompletionGoesByItself() {
        launch(hold: 2)
        openRequest("req-migration")
        openTheCard()
        reveal("todo-row-0")
        drag("todo-row-0", from: 0.08, to: 0.97)
        requireElement("todo-undo")
        XCTAssertTrue(gone("todo-undo", timeout: 10))
        XCTAssertFalse(element("todo-row-6").exists, "and the todo stays done")
    }

    // MARK: adding in place (D6/D7)

    func testTheAddRowTakesOneTodoAfterAnotherAndNeverPicksThem() {
        launch()
        openRequest("req-migration")
        openTheCard()
        XCTAssertTrue(reveal("todo-add-row"))
        element("todo-add-row").tap()
        requireElement("todo-add-field")
        type("Write the migration notes")
        element("todo-add-send").tap()
        requireElement("todo-row-7")
        XCTAssertTrue(element("todo-row-7").label.contains("Write the migration notes"), "last in the queue")
        XCTAssertFalse(element("todo-row-7").isSelected, "adding is not answering")
        XCTAssertFalse(element("todo-pick-count").exists)
        XCTAssertEqual(element("todo-add-field").value as? String ?? "", "", "the input is empty for the next one")

        type("And the rollback plan")
        element("todo-add-send").tap()
        requireElement("todo-row-8")
        XCTAssertTrue(element("todo-row-8").label.contains("And the rollback plan"))
        shot("todo-actions-5-added")
    }

    func testAProjectWithoutTodosHasOneLineThatBecomesTheCard() {
        launch()
        openRequest("req-fresh")
        XCTAssertTrue(reveal("todo-empty-entry"))
        XCTAssertFalse(element("todo-pick-header").exists, "no card for nothing")
        shot("todo-actions-6-empty")
        element("todo-empty-entry").tap()
        requireElement("todo-add-field")
        type("Scaffold the project")
        element("todo-add-send").tap()
        requireElement("todo-row-0")
        XCTAssertTrue(element("todo-row-0").label.contains("Scaffold the project"))
        XCTAssertTrue(element("todo-pick-header").exists, "the line has become the card")
    }

    func testWhatsNextHasTheAddRowAfterShowAllAndAddingOpensTheList() {
        launch()
        openRequest("req-next")
        requireElement("todo-show-all")
        XCTAssertTrue(reveal("todo-add-row"))
        XCTAssertGreaterThan(element("todo-add-row").frame.minY, element("todo-show-all").frame.minY, "after \"Show all\"")
        element("todo-add-row").tap()
        type("A task for later")
        element("todo-add-send").tap()
        requireElement("todo-row-7")
        XCTAssertEqual(requireElement("todo-show-all").label, localized("Show less", "收起"), "the folded list was opened to show it")
        XCTAssertFalse(element("todo-row-7").isSelected)
    }

    // MARK: the Stop confirmation (D10)

    func testTheStopConfirmationListsTheQueueWithTheAddRowBeforeTheFixedChoices() {
        launch()
        openRequest("req-stop-ask")
        requireElement("todo-row-0")
        XCTAssertFalse(element("option-0-0").exists, "the queue stands where the todo the question carried was")
        XCTAssertTrue(reveal("option-0-1"), "\"continue\" follows the queue")
        XCTAssertTrue(element("todo-add-row").exists)
        XCTAssertLessThan(element("todo-add-row").frame.minY, element("option-0-1").frame.minY, "the add row is before the fixed choices")
        XCTAssertLessThan(element("option-0-1").frame.minY, element("option-0-2").frame.minY)
        shot("todo-stop-1")

        element("todo-row-1").tap()
        reveal("todo-row-3")
        element("todo-row-3").tap()
        XCTAssertTrue(element("todo-row-3").isSelected)
        XCTAssertFalse(element("todo-row-1").isSelected, "what comes next is one thing")
        element("option-0-2").tap()
        XCTAssertFalse(element("todo-row-3").isSelected, "ending the conversation takes the todo's place")
        XCTAssertTrue(element("option-0-2").isSelected)
    }

    func testTheStopConfirmationCompletesAndAddsToo() {
        launch()
        openRequest("req-stop-ask")
        requireElement("todo-row-0")
        drag("todo-row-0", from: 0.08, to: 0.97)
        requireElement("todo-undo")
        XCTAssertEqual(requireElement("todo-show-all").label, localized("Show all 6 todos", "显示全部 6 条待办"), "seven less the one that was done")
        XCTAssertTrue(element("todo-row-0").label.contains(texts[1]))
        XCTAssertTrue(reveal("todo-add-row"))
        element("todo-add-row").tap()
        type("Check the Stop card")
        element("todo-add-send").tap()
        XCTAssertEqual(requireElement("todo-show-all").label, localized("Show less", "收起"), "adding opened the list")
        XCTAssertTrue(reveal("todo-row-6"))
        XCTAssertTrue(element("todo-row-6").label.contains("Check the Stop card"))
    }
}
