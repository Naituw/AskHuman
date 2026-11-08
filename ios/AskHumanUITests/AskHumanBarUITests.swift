import XCTest

/// The bar at the foot of a question form (`docs/specs/ios-answer-flow.md` §3.4) in the states that crowd it, at the text
/// sizes that make a phone narrow. The buttons never get taller than they are at rest, and the count (which gives way to
/// them) stays whole and clear of them, whether it sits beside them or above them. In English or Chinese
/// (`TEST_RUNNER_AH_UI_LANG=zh-Hans`).
///
/// What narrows the bar is the room: a larger text size on the phone this runs on stands in for a smaller phone. On a
/// 375 pt phone the default size already crowds the last two states.
@MainActor
final class AskHumanBarUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }
    /// The text size the run is in (`UICTContentSizeCategory…`), for the names of what it saves.
    private var sizeName = "default"
    /// What the screen held when the menu did not show its entries, for the failure message.
    private var menuTrouble = ""
    /// How many notes the run's request has on its message: with some, the bar has the round "Annotations" button too
    /// (`docs/specs/message-annotations.md` D21).
    private var notes = 0

    override func setUp() async throws {
        // `TEST_RUNNER_AH_UI_CONTINUE=1` measures every state even after one assertion failed (a run that is there to
        // collect frames, not to gate anything).
        continueAfterFailure = ProcessInfo.processInfo.environment["AH_UI_CONTINUE"] == "1"
    }

    override func tearDown() async throws {
        if let run = testRun, run.totalFailureCount > 0, app != nil { shot("failed-" + name.filter { $0.isLetter }) }
    }

    // MARK: helpers

    /// Opens the three-question migration request on a Mac that knows away mode, so that the bar can hold all it ever
    /// holds: the count, the quick-reply menu, the round "Annotations" button (when `notes` says there are notes),
    /// "Next" beside "Send".
    @discardableResult
    private func launch(contentSize: String? = nil, notes: Int = 0) -> XCUIApplication {
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", "away", "-AHOpen", "req-migration", "-AHLatency", "0.05", "-AHAutoAdvance", "NO",
            "-AHWebMarkdown", "NO", "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ]
        if notes > 0 {
            // The Macs of the away scenario take no notes: this makes them (and the request) take them, and writes some.
            app.launchArguments += ["-AHAnnotate", "YES", "-AHNotes", "\(notes)"]
        }
        self.notes = notes
        if let contentSize {
            app.launchArguments += ["-UIPreferredContentSizeCategoryName", contentSize]
            sizeName = contentSize.replacingOccurrences(of: "UICTContentSizeCategory", with: "")
        }
        if notes > 0 { sizeName += "-notes" }
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
            let url = URL(fileURLWithPath: folder).appendingPathComponent("\(language)-\(sizeName)-\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
            return
        }
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = "\(language)-\(sizeName)-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// The frames of what is on the bar right now, one line per state, appended to `frames.txt` in the folder of the
    /// screenshots (when there is one): what a screenshot cannot say exactly.
    private func dump(_ state: String) {
        guard let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"], !folder.isEmpty else { return }
        func rect(_ id: String) -> String {
            let found = element(id)
            guard found.exists else { return "-" }
            let f = found.frame
            return String(format: "%.0f,%.0f %.0fx%.0f", f.minX, f.minY, f.width, f.height)
        }
        let window = app.windows.firstMatch.frame
        let line = "\(language)-\(sizeName) \(state): window \(Int(window.width))x\(Int(window.height)) | count \(rect("answered-count")) | "
            + "menu \(rect("quick-reply-menu")) | notes \(rect("annotations-button")) | next \(rect("next-button")) | "
            + "send \(rect("send-button"))\n"
        let url = URL(fileURLWithPath: folder).appendingPathComponent("frames.txt")
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        if let handle = try? FileHandle(forWritingTo: url) {
            handle.seekToEndOfFile()
            handle.write(Data(line.utf8))
            try? handle.close()
        } else {
            try? Data(line.utf8).write(to: url)
        }
    }

    private func scrollToTop() {
        for _ in 0..<10 { app.swipeDown(velocity: .fast) }
        Thread.sleep(forTimeInterval: 0.5)
    }

    /// Scrolls until the last question's field is on screen (and "Send" has taken the place of "Next").
    private func reachTheEnd() {
        var swipes = 0
        while !element("send-button").exists, swipes < 12 {
            app.swipeUp(velocity: .slow)
            swipes += 1
        }
        XCTAssertTrue(element("send-button").waitForExistence(timeout: 5), "Send should show once the last question was on screen")
    }

    /// Looks at the bar in the state the form is in and checks what must hold in every one; returns the height of the
    /// main button, which the next states are held to.
    ///
    /// - Parameters:
    ///   - primaryID: the main button of the state: "Next" until the end of the form was reached, then "Send".
    ///   - restingHeight: the height the main button had in the first state.
    ///   - circles: the quick-reply menu and "Next" beside the main button are circles as tall as it (the floating bar; the
    ///     classic bar's buttons are not made one height).
    @discardableResult
    private func checkTheBar(_ state: String, primaryID: String, restingHeight: CGFloat? = nil, circles: Bool = true) -> CGFloat {
        Thread.sleep(forTimeInterval: 0.8)
        shot(state)
        dump(state)

        let window = app.windows.firstMatch.frame
        let count = requireElement("answered-count").frame
        let primary = requireElement(primaryID).frame
        let buttons: [(id: String, frame: CGRect)] = ["quick-reply-menu", "annotations-button", "next-button", "send-button"].compactMap { id in
            let found = element(id)
            return found.exists ? (id, found.frame) : nil
        }
        // The notes are in the draft from the start of the run, so their button is there in every state, with their number.
        XCTAssertEqual(element("annotations-button").exists, notes > 0, "\(state): the Annotations button is there when there are notes, and only then")
        if notes > 0 {
            XCTAssertTrue(
                element("annotations-button").label.contains("\(notes)"), "\(state): the button says how many notes there are"
            )
        }

        // The point of it all: what is said beside the buttons never makes them taller than they are at rest. (Shorter is
        // allowed: "Send and go away" is scaled down at the larger sizes, and its button is as tall as that text.)
        if let restingHeight {
            XCTAssertLessThanOrEqual(primary.height, restingHeight + 1, "\(state): the main button was stretched")
        }
        // The quick-reply menu and "Next" beside the main button are in its row: circles, as tall as it (so all of the same
        // size, whatever the text size). A "Next" that is the main button itself is a capsule, and is left out.
        if circles {
            for button in buttons where button.id != primaryID {
                XCTAssertEqual(button.frame.height, primary.height, accuracy: 1, "\(state): \(button.id) and the main button are not one height")
                XCTAssertEqual(button.frame.width, button.frame.height, accuracy: 1, "\(state): \(button.id) is not round")
            }
        }
        // The count gave way to the buttons, but not down to nothing (an empty chip is the padding alone, 28 pt). The floating
        // bar says "0/3" when "Answered 0/3" does not fit beside the buttons, and both bars put the count above them when not
        // even that does. The classic bar has no short wording: its count is one whole line, beside the buttons or above them
        // (what that replaced was "An-/swered 0/3" on three lines, and "A" with "Send and go away" armed).
        XCTAssertGreaterThanOrEqual(count.width, 40, "\(state): the count was squeezed out")
        if !circles {
            XCTAssertGreaterThan(count.width, count.height * 2.5, "\(state): the count is wrapped")
        }
        XCTAssertGreaterThan(count.height, 0, "\(state): the count has no height")
        XCTAssertGreaterThanOrEqual(count.minX, 0, "\(state): the count is off the screen")
        XCTAssertLessThanOrEqual(count.maxX, window.width + 0.5, "\(state): the count is off the screen")
        for button in buttons {
            XCTAssertGreaterThanOrEqual(button.frame.minX, 0, "\(state): \(button.id) is off the screen")
            XCTAssertLessThanOrEqual(button.frame.maxX, window.width + 0.5, "\(state): \(button.id) is off the screen")
            XCTAssertFalse(count.intersects(button.frame), "\(state): the count overlaps \(button.id)")
        }
        for (index, first) in buttons.enumerated() {
            for second in buttons[(index + 1)...] {
                XCTAssertFalse(first.frame.intersects(second.frame), "\(state): \(first.id) overlaps \(second.id)")
            }
        }
        return primary.height
    }

    /// Drags on the open quick-reply menu, from a point above its button where the menu is. Not `swipeUp()` / `swipeDown()` on
    /// the app: those start in the middle of the screen, which is outside a menu that sits low, and a touch that starts
    /// outside the menu closes it. Dragging down brings the far end (the instructions) into view, dragging up the near end.
    private func scrollTheMenu(towardsTheFarEnd: Bool) {
        let button = element("quick-reply-menu").frame
        let from = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: button.midX, dy: max(button.minY - 120, 80)))
        from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: towardsTheFarEnd ? 320 : -320)))
    }

    /// The bar floats from iOS 26 on; before that it is the classic bar, whose buttons are not made one height.
    private var barFloats: Bool { ProcessInfo.processInfo.operatingSystemVersion.majorVersion >= 26 }

    /// Takes the form through the three states that crowd the bar: "Next" with the menu; "Send" with "Next" and the menu
    /// (after the end was reached and the form is back at the top, where "Next" is still of use); and "Send and go away".
    private func checkTheCrowdedStates() {
        let circles = barFloats
        requireElement("next-button")
        Thread.sleep(forTimeInterval: 1.0)
        let resting = checkTheBar("1-top", primaryID: "next-button", circles: circles)

        reachTheEnd()
        scrollToTop()
        requireElement("next-button")
        XCTAssertTrue(element("send-button").exists, "Send does not go away once the end was reached")
        checkTheBar("2-send-and-next", primaryID: "send-button", restingHeight: resting, circles: circles)

        requireElement("quick-reply-menu").tap()
        // The system gives a menu's entries no identifiers: this one is found by its text. At the larger text sizes the menu is
        // longer than what the system shows of it. It opens upward from the bar, with the quick replies (the last to be
        // declared) nearest the thumb and the instructions at the far end, above the fold, and only the rows on screen are
        // there to be found: the menu has to be scrolled to bring them into view.
        let away = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", localized("Away mode", "离开模式"))).firstMatch
        if !away.waitForExistence(timeout: 6) {
            for attempt in 0..<4 where !away.exists {
                scrollTheMenu(towardsTheFarEnd: attempt % 2 == 0)
                _ = away.waitForExistence(timeout: 2)
            }
        }
        if !away.exists {
            // Evidence: what was on the screen when the entry was not there.
            shot("menu-missing-\(Int(Date().timeIntervalSince1970))")
            menuTrouble = "menus \(app.menus.count), menu items \(app.menuItems.count), buttons "
                + app.buttons.allElementsBoundByIndex.prefix(14).map(\.label).joined(separator: "|")
        }
        XCTAssertTrue(away.exists, "the menu offers away mode (\(menuTrouble))")
        away.tap()
        requireElement("directive-panel")
        checkTheBar("3-armed", primaryID: "send-button", restingHeight: resting, circles: circles)
    }

    // MARK: the crowded states, by text size

    func testTheBarAtTheDefaultTextSize() {
        launch()
        checkTheCrowdedStates()
    }

    func testTheBarAtLargeText() {
        launch(contentSize: "UICTContentSizeCategoryXL")
        checkTheCrowdedStates()
    }

    func testTheBarAtLargerText() {
        launch(contentSize: "UICTContentSizeCategoryXXL")
        checkTheCrowdedStates()
    }

    func testTheBarAtTheLargestStandardText() {
        launch(contentSize: "UICTContentSizeCategoryXXXL")
        checkTheCrowdedStates()
    }

    /// The accessibility sizes stack the count over the buttons whatever the width is.
    func testTheBarAtAccessibilityText() {
        launch(contentSize: "UICTContentSizeCategoryAccessibilityM")
        checkTheCrowdedStates()
    }

    // MARK: with notes on the message: a round "Annotations" button joins the row

    // The same three states, with the third circle (the quick-reply menu, "Annotations", "Next") in the row. How much room
    // that leaves the count and the main button on a 375 pt phone is what `docs/specs/message-annotations.md` §4.3 only
    // estimates: these are run on that width (docs/plans/message-annotations.md §7.3).

    func testTheBarWithNotesAtTheDefaultTextSize() {
        launch(notes: 2)
        checkTheCrowdedStates()
    }

    /// The most notes there are: the badge is two digits wide.
    func testTheBarWithTheMostNotes() {
        launch(notes: 20)
        checkTheCrowdedStates()
    }

    func testTheBarWithNotesAtLargeText() {
        launch(contentSize: "UICTContentSizeCategoryXL", notes: 2)
        checkTheCrowdedStates()
    }

    func testTheBarWithNotesAtLargerText() {
        launch(contentSize: "UICTContentSizeCategoryXXL", notes: 2)
        checkTheCrowdedStates()
    }

    func testTheBarWithNotesAtTheLargestStandardText() {
        launch(contentSize: "UICTContentSizeCategoryXXXL", notes: 2)
        checkTheCrowdedStates()
    }

    func testTheBarWithNotesAtAccessibilityText() {
        launch(contentSize: "UICTContentSizeCategoryAccessibilityM", notes: 2)
        checkTheCrowdedStates()
    }
}
