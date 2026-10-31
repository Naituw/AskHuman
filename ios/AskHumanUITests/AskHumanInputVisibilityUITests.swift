import XCTest

/// The real insertion point and keyboard, over the mock backend without starting an Agent.
@MainActor
final class AskHumanInputVisibilityUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() async throws { continueAfterFailure = false }

    private func launch(_ arguments: [String]) {
        app = XCUIApplication()
        app.launchArguments = [
            "-AHMock", "YES", "-AHLatency", "0", "-AHWebMarkdown", "NO",
            "-AHInputCaretDiagnostics", "YES", "-AppleLanguages", "(en)", "-AppleLocale", "en_US",
        ] + arguments
        app.launch()
    }

    private func field(_ identifier: String) -> XCUIElement {
        for _ in 0..<10 {
            let element = app.textViews[identifier]
            if element.exists, element.isHittable { return element }
            app.swipeUp(velocity: .fast)
        }
        let element = app.textViews[identifier]
        XCTAssertTrue(element.waitForExistence(timeout: 10))
        return element
    }

    private func geometry(_ identifier: String) -> [String: Double] {
        let probe = app.descendants(matching: .any).matching(identifier: identifier + "-caret-state").firstMatch
        guard let value = probe.value as? String else { return [:] }
        return Dictionary(uniqueKeysWithValues: value.split(separator: ";").compactMap { pair in
            let parts = pair.split(separator: "=", maxSplits: 1)
            guard parts.count == 2, let number = Double(parts[1]) else { return nil }
            return (String(parts[0]), number)
        })
    }

    private func assertCaretVisible(_ identifier: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), file: file, line: line)
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate { [self] _, _ in
            let measured = geometry(identifier)
            guard let top = measured["top"], let bottom = measured["bottom"],
                  let visibleTop = measured["visibleTop"], let visibleBottom = measured["visibleBottom"],
                  let clipTop = measured["clipTop"], let clipBottom = measured["clipBottom"] else { return false }
            let keyboardTop = app.keyboards.firstMatch.frame.minY
            return top >= max(visibleTop, clipTop) - 1 && bottom <= clipBottom + 1
                && bottom + 8 <= min(visibleBottom, keyboardTop) + 1
        }, object: app)
        XCTAssertEqual(XCTWaiter().wait(for: [expectation], timeout: 10), .completed, "\(geometry(identifier))", file: file, line: line)
    }

    private func typeLines(_ identifier: String) {
        let editor = field(identifier)
        editor.tap()
        for batch in 0..<4 {
            editor.typeText((1...5).map { "Line \(batch * 5 + $0) with a note" }.joined(separator: "\n") + "\n")
            assertCaretVisible(identifier)
        }
        XCTAssertTrue((editor.value as? String)?.contains("Line 20 with a note") == true)
    }

    func testAnswerCaretStaysAboveTheKeyboardAcrossNewlinesAndInternalScrolling() {
        launch(["-AHOpen", "req-next"])
        typeLines("answer-text-0")
        attachScreenshot("answer-multiline")
    }

    private func assertAttachmentVisible(_ question: Int, file: StaticString = #filePath, line: UInt = #line) {
        let button = app.buttons["add-attachment-\(question)"]
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate { [self] _, _ in
            let measured = geometry("answer-text-\(question)")
            guard let top = measured["visibleTop"], let bottom = measured["visibleBottom"] else { return false }
            return button.exists && button.isHittable && button.frame.minY >= top
                && button.frame.maxY + 8 <= min(bottom, app.keyboards.firstMatch.frame.minY) + 1
        }, object: app)
        XCTAssertEqual(XCTWaiter().wait(for: [expectation], timeout: 20), .completed,
                       "attachment: \(button.frame), hittable: \(button.isHittable), keyboard: \(app.keyboards.firstMatch.frame), input: \(geometry("answer-text-\(question)"))", file: file, line: line)
    }

    func testFocusingAnEmptyAnswerRevealsTheAttachmentMenu() {
        launch(["-AHOpen", "req-next"])
        field("answer-text-0").tap()
        assertCaretVisible("answer-text-0")
        assertAttachmentVisible(0)
        attachScreenshot("answer-empty-attachment")
        app.buttons["add-attachment-0"].tap()
        XCTAssertTrue(app.buttons["Choose File"].waitForExistence(timeout: 5))
    }

    func testShortAnswerKeepsTheAttachmentMenuVisibleAcrossNewlines() {
        launch(["-AHOpen", "req-next"])
        let editor = field("answer-text-0")
        editor.tap()
        editor.typeText("A short answer\nOne more line\n")
        assertCaretVisible("answer-text-0")
        assertAttachmentVisible(0)
        attachScreenshot("answer-short-attachment")
    }

    func testShortAnswerAndAttachmentFitWithAccessibilityText() {
        launch(["-AHOpen", "req-next", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityL"])
        let editor = field("answer-text-0")
        editor.tap()
        editor.typeText("A note\n")
        assertCaretVisible("answer-text-0")
        assertAttachmentVisible(0)
        attachScreenshot("answer-accessibility-attachment")
    }

    func testEachQuestionRevealsItsOwnAttachmentEntryPoint() {
        launch(["-AHOpen", "req-migration"])
        let first = field("answer-text-0")
        first.tap()
        first.typeText("First answer")
        assertCaretVisible("answer-text-0")
        assertAttachmentVisible(0)
        let second = field("answer-text-1")
        second.tap()
        second.typeText("Second answer")
        assertCaretVisible("answer-text-1")
        assertAttachmentVisible(1)
        XCTAssertEqual(first.value as? String, "First answer")
        attachScreenshot("answer-second-question-attachment")
    }

    /// The isolated simulator must contain a photo, as seeded with `simctl addmedia`.
    func testAttachmentRowsMoveTheEntryPointWithoutHidingTheCaret() throws {
        guard ProcessInfo.processInfo.environment["AH_INPUT_ATTACHMENT_QA"] == "YES" else {
            throw XCTSkip("Requires the isolated attachment fixture library")
        }
        launch(["-AHOpen", "req-next"])
        let editor = field("answer-text-0")
        editor.tap()
        assertAttachmentVisible(0)
        app.buttons["add-attachment-0"].tap()
        app.buttons["Photo Library"].tap()
        let photo = app.images.matching(NSPredicate(format: "label BEGINSWITH 'Photo,'")).firstMatch
        XCTAssertTrue(photo.waitForExistence(timeout: 30))
        photo.tap()
        let done = app.buttons["Done"]
        if done.exists { done.tap() } else { app.buttons["Add"].tap() }
        XCTAssertTrue(app.buttons["Remove attachment"].waitForExistence(timeout: 15))
        editor.tap()
        editor.typeText("An attached photo")
        assertCaretVisible("answer-text-0")
        assertAttachmentVisible(0)
        attachScreenshot("answer-existing-attachment")
        app.buttons["Remove attachment"].tap()
        assertCaretVisible("answer-text-0")
        assertAttachmentVisible(0)
    }

    func testTaskCaretStaysAboveTheKeyboardAcrossNewlinesAndInternalScrolling() {
        launch(["-AHNewTask", "YES"])
        typeLines("task-text")
        attachScreenshot("task-multiline")
    }

    func testNaturalWrappingAndEditingAboveTheEndFollowTheCurrentCaret() {
        launch(["-AHOpen", "req-next"])
        let identifier = "answer-text-0"
        let editor = field(identifier)
        editor.tap()
        editor.typeText(String(repeating: "This wraps without pressing return. ", count: 16))
        assertCaretVisible(identifier)
        let measured = geometry(identifier)
        // The top of the visible part of this long editor is earlier than its final line.
        let y = max(measured["visibleTop"] ?? 0, editor.frame.minY) + 35
        app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: editor.frame.midX, dy: y)).tap()
        let before = geometry(identifier)
        XCTAssertLessThan(before["location"] ?? .infinity, before["length"] ?? 0)
        editor.typeText(" added\n")
        assertCaretVisible(identifier)
        let after = geometry(identifier)
        XCTAssertLessThan(after["location"] ?? .infinity, after["length"] ?? 0, "editing must not move to the end")
    }

    private func attachScreenshot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testManualScrollingStaysPutUntilTypingResumes() {
        launch(["-AHOpen", "req-next"])
        let identifier = "answer-text-0"
        let editor = field(identifier)
        editor.tap()
        editor.typeText((1...25).map { "Line \($0)" }.joined(separator: "\n"))
        assertCaretVisible(identifier)
        let location = geometry(identifier)["location"]
        editor.swipeDown(velocity: .slow)
        let inside = XCTNSPredicateExpectation(predicate: NSPredicate { [self] _, _ in
            let measured = geometry(identifier)
            guard let bottom = measured["bottom"], let clipBottom = measured["clipBottom"] else { return false }
            return bottom <= clipBottom + 1
        }, object: app)
        inside.isInverted = true
        XCTAssertEqual(XCTWaiter().wait(for: [inside], timeout: 1), .completed, "reading earlier text must not snap back to the caret")
        XCTAssertEqual(geometry(identifier)["location"], location)
        editor.typeText(" more")
        assertCaretVisible(identifier)
    }


}
