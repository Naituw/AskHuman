import XCTest

/// The options of a question are as tall as their text, whatever page they are on (`docs/specs/ios-session-history.md` §7.3,
/// `requestRoom`). The `optionWrap` scenario has two questions whose options break lines on purpose, so that the lines do
/// not depend on the width of the screen: the first has five options of one line and two of two lines, the second three of
/// three lines. The first card is the taller one and the one that can give up less height, which is the shape in which a
/// page that handed its content a height of its own took the lines out of the options of the first card (they were left
/// cut to one line, with an ellipsis, while the second card showed all of its lines).
@MainActor
final class AskHumanOptionLayoutUITests: XCTestCase {
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
    private func launch(scenario: String = "optionWrap") -> XCUIApplication {
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

    private func waitUntil(timeout: TimeInterval = 6, _ condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return true }
            Thread.sleep(forTimeInterval: 0.2)
        }
        return condition()
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

    // MARK: the options of a card that is taller and less flexible than the next

    func testOptionsThatBreakLinesKeepAllTheirLines() {
        launch()
        requireElement("inbox-row-req-wrap").tap()
        let oneLine = requireElement("option-0-0").frame.height
        // A row of one line is 44 pt; every further line adds about 20 pt. The thresholds sit between the height of a row
        // that shows its lines and the one it falls to when a line is taken away (a row cut to one line is 44 pt, one cut
        // from three lines to two is about 62 pt).
        let wrapped: [(id: String, lines: Int)] = [
            ("option-0-5", 2), ("option-0-6", 2), ("option-1-0", 3), ("option-1-1", 3), ("option-1-2", 3),
        ]
        func tallEnough(_ row: (id: String, lines: Int)) -> Bool {
            element(row.id).frame.height > oneLine + CGFloat(row.lines - 1) * 14
        }
        let shown = waitUntil { wrapped.allSatisfy(tallEnough) }
        shot("option-wrap")
        let heights = wrapped.map { "\($0.id) \($0.lines) lines: \(element($0.id).frame.height)" }.joined(separator: ", ")
        XCTAssertTrue(shown, "an option lost lines (one line is \(oneLine) pt): \(heights)")
    }
}
