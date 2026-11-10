import XCTest

/// The body of a request is drawn by a web page that runs the Mac popup's Markdown engine. These tests
/// open the mock's Markdown showcase (`-AHScenario markdown`) and look at what the page makes of it, and
/// at the native renderer that takes over when the page is switched off (`-AHWebMarkdown NO`).
///
/// The page's content shows up in the accessibility tree as ordinary elements, with no web view around
/// them, so the tests tell the two renderers apart by what only one of them draws: the page has a button
/// labelled "Copy" on every code block and a picture of the diagram, the native renderer has "Copy code"
/// and the diagram's source.
///
/// English only: the menus and dialogs they look for are found by their English names.
@MainActor
final class AskHumanWebUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() async throws {
        continueAfterFailure = false
    }

    // MARK: helpers

    /// `webTimeout` is long because the simulator takes seconds to start web content the first time.
    @discardableResult
    private func launch(
        _ scenario: String = "markdown", language: String = "en", webTimeout: Double = 60, extra: [String] = []
    ) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = [
            "-AHMock", "YES", "-AHScenario", scenario, "-AHLatency", "0.05", "-AHAutoAdvance", "NO", "-AHWebTimeout", "\(webTimeout)",
            "-AppleLanguages", "(\(language))", "-AppleLocale", language == "en" ? "en_US" : "zh_CN",
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

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"], !folder.isEmpty {
            let url = URL(fileURLWithPath: folder).appendingPathComponent("\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
            return
        }
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func openShowcase(_ id: String = "req-markdown") {
        requireElement("inbox-row-\(id)").tap()
        _ = requireElement("request-title")
    }

    /// An element on the screen whose label contains the text.
    private func labelled(_ text: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    /// The picture of the Mermaid diagram: only the page draws it, with the words in it as its label.
    private var diagram: XCUIElement {
        app.images.matching(NSPredicate(format: "label CONTAINS %@", "收到变更请求")).firstMatch
    }

    /// The page's copy button of a code block (the native renderer's is labelled "Copy code").
    private var pageCopyButton: XCUIElement {
        app.buttons.matching(NSPredicate(format: "label == 'Copy'")).firstMatch
    }

    // MARK: the page

    func testTheMessageIsDrawnByTheWebPage() {
        launch()
        openShowcase()
        XCTAssertTrue(diagram.waitForExistence(timeout: 60), "the page draws the diagram")
        XCTAssertTrue(app.staticTexts["迁移评估报告"].exists, "the heading is on the page")
        XCTAssertTrue(labelled("在线双写").exists, "so is the table")
        XCTAssertFalse(app.buttons["Copy code"].exists, "none of it is drawn by the native renderer")
        shot("30-web-top")
        app.swipeUp(velocity: .slow)
        Thread.sleep(forTimeInterval: 0.8)
        shot("30b-web-diagram")
    }

    func testTheNativeRendererDrawsTheMessageWhenThePageIsSwitchedOff() {
        launch(extra: ["-AHWebMarkdown", "NO"])
        openShowcase()
        XCTAssertTrue(app.staticTexts["迁移评估报告"].waitForExistence(timeout: 15), "the heading is there")
        XCTAssertFalse(diagram.exists, "the diagram stays as its source")
        XCTAssertFalse(pageCopyButton.exists)
        shot("31-native-top")
    }

    /// A page that does not get ready in time is given up on: the native renderer draws the message
    /// (the watchdog is shortened to a blink so that no page can make it).
    func testAPageThatIsTooSlowGivesWayToTheNativeRenderer() {
        launch(webTimeout: 0.05)
        openShowcase()
        XCTAssertTrue(app.staticTexts["迁移评估报告"].waitForExistence(timeout: 15), "the heading is there all the same")
        XCTAssertTrue(app.buttons["Copy code"].waitForExistence(timeout: 10), "drawn by the native renderer")
        XCTAssertFalse(diagram.exists)
        XCTAssertFalse(pageCopyButton.exists)
        // And the form is usable: Next is offered, and it leads on.
        XCTAssertTrue(element("next-button").exists)
    }

    func testALinkOnThePageAsksBeforeItOpens() {
        launch()
        openShowcase()
        XCTAssertTrue(diagram.waitForExistence(timeout: 60), "the page is drawn")
        // The link to a bare address (a link with words around it can wrap, which makes its frame a box).
        let link = app.links.matching(NSPredicate(format: "label == %@", "https://example.com/docs")).firstMatch
        XCTAssertTrue(link.waitForExistence(timeout: 10), "the link is on the page")
        let frame = link.frame
        link.tap()
        let open = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "example.com")).firstMatch
        requireOrShow(open, "the person is asked, with the domain, before the link opens (link frame \(frame))", slug: "page-link", timeout: 5)
        shot("32-link-asks")
    }

    // MARK: links of other kinds (docs/specs/ios-web-markdown.md W7)
    //
    // The `links` scenario has a link of every kind, each on a line of its own. A web address is asked about with its
    // domain; any other scheme is asked about with the whole address and handed to whichever app has claimed it (there
    // is no list of schemes behind that); a scheme that nobody has claimed is said, not dropped.

    private func openLinks() {
        launch("links")
        openShowcase("req-links")
        XCTAssertTrue(link("官方文档").waitForExistence(timeout: 60), "the page is drawn")
    }

    /// The link of the page that has exactly these words (a link of one line, whose frame is the line).
    private func link(_ words: String) -> XCUIElement {
        app.links.matching(NSPredicate(format: "label == %@", words)).firstMatch
    }

    /// The distance between two rectangles: nothing when they touch or overlap.
    private func gap(_ a: CGRect, _ b: CGRect) -> CGFloat {
        hypot(max(a.minX - b.maxX, b.minX - a.maxX, 0), max(a.minY - b.maxY, b.minY - a.maxY, 0))
    }

    /// Puts the question away without answering it: its Cancel button where there is one (a popover has none: a tap
    /// outside it is how it goes away).
    private func dismissTheQuestion() {
        if app.buttons["Cancel"].exists {
            app.buttons["Cancel"].tap()
        } else {
            // A spot that is blank under the popover (the sidebar below its list); the popover's dimming view takes the tap.
            app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.12, dy: 0.85)).tap()
        }
        let gone = NSPredicate(format: "exists == false")
        for title in ["Open this link?", "Open in another app?"] {
            let wait = XCTNSPredicateExpectation(predicate: gone, object: app.staticTexts[title])
            let result = XCTWaiter().wait(for: [wait], timeout: 15)
            if result != .completed { shot("49-question-not-put-away") }
            XCTAssertEqual(result, .completed, "the question '\(title)' is put away:\n\(app.debugDescription)")
        }
    }

    func testALinkToAnotherAppShowsTheWholeAddressBeforeItLeaves() {
        openLinks()
        link("在地图里搜索").tap()
        XCTAssertTrue(app.staticTexts["Open in another app?"].waitForExistence(timeout: 5), "the person is asked first")
        XCTAssertTrue(app.staticTexts["maps://?q=AskHuman"].exists, "with the whole address, not a guess at the app")
        shot("46-link-app-asks")
        dismissTheQuestion()
        XCTAssertEqual(app.state, .runningForeground, "not following it leaves the app where it was")
    }

    /// Waits for `element`; when it does not come, the screen and the tree are kept for whoever reads the failure.
    private func requireOrShow(
        _ element: XCUIElement, _ message: String, slug: String, timeout: TimeInterval = 15,
        file: StaticString = #filePath, line: UInt = #line
    ) {
        let came = element.waitForExistence(timeout: timeout)
        if !came { shot("missing-\(slug)") }
        XCTAssertTrue(came, "\(message):\n\(app.debugDescription)", file: file, line: line)
    }

    /// A point a little in from the left edge of an element's frame, on the first words of a line of text: the link
    /// of the native renderer's text is the first words, and the middle of its frame can be past the end of the text.
    private func startOf(_ element: XCUIElement) -> XCUICoordinate {
        element.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0.5)).withOffset(CGVector(dx: 24, dy: 0))
    }

    /// The native renderer takes the same links (SwiftUI's `openURL`): the question of this request is drawn by it,
    /// and begins with a link.
    func testALinkInTheQuestionIsAskedAboutToo() {
        openLinks()
        let question = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "地图里的搜索")).firstMatch
        XCTAssertTrue(question.waitForExistence(timeout: 10), "the question is on the page:\n\(app.debugDescription)")
        shot("50-link-in-question-before")
        let frame = question.frame
        startOf(question).tap()
        requireOrShow(app.staticTexts["Open in another app?"], "the person is asked here too (question frame \(frame))", slug: "question-link")
        XCTAssertTrue(app.staticTexts["maps://?q=AskHuman"].exists, "with the whole address")
        shot("50-link-in-question")
        dismissTheQuestion()
    }

    /// A link in a file that is open in a sheet is asked about over that sheet: the question is presented from
    /// whatever is on top (one hung on the root of the app has nowhere to come up while a sheet covers it).
    func testALinkInAnAttachmentIsAskedAboutOverItsSheet() {
        openLinks()
        let file = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "links-note.md")).firstMatch
        XCTAssertTrue(file.waitForExistence(timeout: 10), "the file is listed:\n\(app.debugDescription)")
        file.tap()
        let link = labelled("附件里的链接")
        XCTAssertTrue(link.waitForExistence(timeout: 30), "the file is open in its sheet:\n\(app.debugDescription)")
        startOf(link).tap()
        XCTAssertTrue(app.staticTexts["Open this link?"].waitForExistence(timeout: 15), "the question comes up over the sheet")
        shot("51-link-over-sheet")
        dismissTheQuestion()
        XCTAssertTrue(link.exists, "the sheet is still open under it")
    }

    /// Once confirmed, a link of an app's scheme is handed to the system, with no list of schemes in between: the
    /// app that claims `maps://` (the simulator has Maps) is brought to the front, and this one goes behind it.
    func testAConfirmedLinkIsHandedToTheAppThatClaimsItsScheme() {
        let maps = XCUIApplication(bundleIdentifier: "com.apple.Maps")
        addTeardownBlock { maps.terminate() }
        openLinks()
        link("在地图里搜索").tap()
        XCTAssertTrue(app.staticTexts["Open in another app?"].waitForExistence(timeout: 10), "the person is asked first")
        app.buttons["Open"].tap()
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 90), "this app went behind the one that took the link")
        XCTAssertTrue(maps.wait(for: .runningForeground, timeout: 30), "and that one is in front")
        app.activate()
    }

    /// A scheme that no app has claimed is no dead end that goes unnoticed: the person is told, with the address.
    func testALinkNoAppHasClaimedIsSaidNotDropped() {
        openLinks()
        link("未安装的 App").tap()
        XCTAssertTrue(app.staticTexts["Open in another app?"].waitForExistence(timeout: 5))
        app.buttons["Open"].tap()
        XCTAssertTrue(labelled("No app can open this link").waitForExistence(timeout: 10), "the person is told")
        XCTAssertTrue(labelled("ahdemo-unclaimed://open/panel").exists, "which address it was")
        XCTAssertEqual(app.state, .runningForeground, "and was not taken anywhere")
        shot("47-link-unclaimed")
    }

    /// The question and the notice are in the phone's language.
    func testTheQuestionAndTheNoticeSpeakThePhonesLanguage() {
        launch("links", language: "zh-Hans")
        openShowcase("req-links")
        XCTAssertTrue(link("未安装的 App").waitForExistence(timeout: 60), "the page is drawn")
        link("未安装的 App").tap()
        XCTAssertTrue(app.staticTexts["用其他 App 打开？"].waitForExistence(timeout: 10), "the question is in Chinese")
        app.buttons["打开"].tap()
        XCTAssertTrue(labelled("没有 App 可以打开这个链接").waitForExistence(timeout: 10), "so is the notice")
        shot("52-links-chinese")
    }

    /// What is not made a link is not offered as one: a script, and a bare address that is not a web one.
    func testWhatIsNotFollowedIsNotALink() {
        openLinks()
        XCTAssertFalse(link("脚本").exists, "a script is no link")
        XCTAssertTrue(labelled("javascript:alert(1)").exists, "it stays as the words it was written with")
        XCTAssertFalse(app.links.matching(NSPredicate(format: "label CONTAINS %@", "ahdemo-unclaimed://bare")).firstMatch.exists)
    }

    /// Where there is room beside the link, the question is a popover that points at the link that was tapped (not at
    /// the screen's middle, and not at the edge of the window).
    func testTheQuestionPointsAtTheLinkOnAWideScreen() throws {
        openLinks()
        try XCTSkipIf(app.windows.firstMatch.frame.width < 600, "A phone's question comes up from the bottom edge.")
        for words in ["官方文档", "在地图里搜索"] {
            let target = link(words)
            XCTAssertTrue(target.waitForExistence(timeout: 5))
            let tapped = target.frame
            target.tap()
            let popover = app.popovers.firstMatch
            XCTAssertTrue(popover.waitForExistence(timeout: 5), "the question is a popover:\n\(app.debugDescription)")
            shot("48-link-popover-\(words == "官方文档" ? "web" : "app")")
            XCTAssertLessThan(gap(tapped, popover.frame), 60, "its arrow is at the link '\(words)': \(tapped) vs \(popover.frame)")
            dismissTheQuestion()
        }
    }

    func testTheTextOfTheMessageCanBeSelected() {
        launch()
        openShowcase()
        XCTAssertTrue(diagram.waitForExistence(timeout: 60), "the page is drawn")
        let paragraph = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "第三行也一样")).firstMatch
        XCTAssertTrue(paragraph.waitForExistence(timeout: 10))
        paragraph.press(forDuration: 1.2)
        let copy = app.descendants(matching: .any).matching(NSPredicate(format: "label == 'Copy'")).firstMatch
        XCTAssertTrue(copy.waitForExistence(timeout: 5), "a long press selects text and offers Copy")
        shot("33-selection")
    }

    func testTheDetailOfAConfirmationIsDrawnByTheWebPage() {
        launch("demo")
        requireElement("inbox-row-req-permission").tap()
        XCTAssertTrue(pageCopyButton.waitForExistence(timeout: 60), "the detail's code block has the page's copy button")
        XCTAssertTrue(labelled("git push").exists)
        shot("34-confirm-detail")
    }

    // MARK: reading

    /// While the page has not drawn the message the questions sit right under a one-line placeholder and
    /// look like they are on screen: they must not count as read (Send would then be offered at once).
    func testNothingBelowTheMessageIsReadWhileItIsStillBeingDrawn() {
        launch("tall")
        openShowcase("req-tall")
        XCTAssertTrue(diagram.waitForExistence(timeout: 60), "the page is drawn")
        Thread.sleep(forTimeInterval: 1.5)
        XCTAssertTrue(element("next-button").exists, "the bar leads on")
        XCTAssertFalse(element("send-button").exists, "the questions were not read when they only looked close")
    }

    /// Next steps over a tall page to the questions one at a time, and ends at Send.
    func testNextWalksDownFromATallMessageToTheLastQuestion() {
        launch()
        openShowcase()
        XCTAssertTrue(diagram.waitForExistence(timeout: 60), "the page is drawn")
        Thread.sleep(forTimeInterval: 1.0)
        var taps = 0
        while !element("send-button").exists, taps < 6 {
            let next = requireElement("next-button")
            next.tap()
            taps += 1
            Thread.sleep(forTimeInterval: 0.8)
        }
        XCTAssertTrue(element("send-button").exists, "Send is offered after the last question was reached")
        XCTAssertLessThanOrEqual(taps, 4, "three questions need three steps, not more")
        XCTAssertGreaterThanOrEqual(taps, 3, "no question was skipped")
        shot("36-next-at-the-end")
    }

    // MARK: looks (screenshots to look at)

    /// The page follows the phone's light or dark look: its text, table and diagram are drawn dark.
    func testTheDarkLookIsDrawnOnThePage() {
        XCUIDevice.shared.appearance = .dark
        addTeardownBlock { XCUIDevice.shared.appearance = .light }
        launch()
        openShowcase()
        XCTAssertTrue(diagram.waitForExistence(timeout: 60))
        Thread.sleep(forTimeInterval: 0.8)
        shot("37-dark-top")
        app.swipeUp(velocity: .slow)
        Thread.sleep(forTimeInterval: 0.8)
        shot("38-dark-diagram")
    }

    /// The words the page shows itself (copy, show source) come in the phone's language.
    func testThePageSpeaksThePhonesLanguage() {
        launch(language: "zh-Hans")
        openShowcase()
        let copy = app.buttons.matching(NSPredicate(format: "label == %@", "复制")).firstMatch
        XCTAssertTrue(copy.waitForExistence(timeout: 60), "the page's copy button is in Chinese")
        let source = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "源码")).firstMatch
        XCTAssertTrue(source.waitForExistence(timeout: 10), "so is the button that shows a diagram's source")
        shot("39-chinese")
    }

    func testLargestTextSizeStillReadsOnThePage() {
        launch(extra: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"])
        openShowcase()
        XCTAssertTrue(diagram.waitForExistence(timeout: 60))
        Thread.sleep(forTimeInterval: 1.0)
        shot("35-xxxl-top")
    }

    // MARK: notes on the message (docs/specs/message-annotations.md)
    //
    // The `annotations` scenario has three requests about a session cache. Their message has two paragraphs of a
    // single word ("Redis", "Memcached"), which can be pressed on to select that word: the system menu of the
    // selection, which a person would use, is what these tests go through.

    private func openNotes(_ id: String = "req-notes") {
        launch("annotations")
        openShowcase(id)
    }

    /// The one-word paragraph, whatever the page has put after it (the number of a note on it).
    private func word(_ text: String) -> XCUIElement {
        let pattern = NSRegularExpression.escapedPattern(for: text) + ".{0,3}"
        return app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", pattern)).firstMatch
    }

    /// An entry of the selection menu (or of anything else that is up) by its title.
    private func entry(_ title: String) -> XCUIElement {
        for query in [app.menuItems, app.buttons, app.staticTexts, app.otherElements] {
            let found = query[title]
            if found.exists { return found }
        }
        return app.menuItems[title]
    }

    private func pollFor(_ title: String, seconds: TimeInterval) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            let found = entry(title)
            if found.exists { return found }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return nil
    }

    /// Presses on a word until the menu of its selection shows `title`. The system does not always show the menu by
    /// itself after a long press (the touch can lift before the page has selected the word); then a tap on the
    /// selection does, as it does for a person.
    private func selectWord(_ word: XCUIElement, until title: String) -> XCUIElement? {
        for _ in 1...3 {
            word.press(forDuration: 1.2)
            if let found = pollFor(title, seconds: 8) { return found }
            word.tap()
            if let found = pollFor(title, seconds: 8) { return found }
            // A tap elsewhere puts the menu away before the next try.
            app.navigationBars.firstMatch.tap()
            Thread.sleep(forTimeInterval: 0.5)
        }
        return nil
    }

    /// Waits until the editor sheet has gone (its text box is not on the screen any more).
    private func waitForTheSheetToGo(file: StaticString = #filePath, line: UInt = #line) {
        let gone = NSPredicate(format: "exists == false")
        let wait = XCTNSPredicateExpectation(predicate: gone, object: element("annotation-field"))
        XCTAssertEqual(XCTWaiter().wait(for: [wait], timeout: 10), .completed, "the sheet is still up", file: file, line: line)
    }

    /// A word is selected, the entry is chosen, the note typed and added; the word is a note then, and the menu of
    /// the same word offers to edit it. The note is changed, and then taken away, and the word offers a new one again.
    func testANoteIsWrittenOnASelectedWordAndComesBackForEditing() {
        openNotes()
        XCTAssertTrue(word("Redis").waitForExistence(timeout: 60), "the page is drawn")
        let add = selectWord(word("Redis"), until: "Add annotation")
        XCTAssertNotNil(add, "the selection's menu has the entry")
        shot("40-notes-menu")
        add?.tap()

        let field = requireElement("annotation-field", timeout: 10)
        XCTAssertTrue(element("annotation-quote").label.contains("Redis"), "the sheet shows the passage")
        shot("40b-notes-sheet-empty")
        // Nothing is written yet, so "Add" does nothing. (Asked by pressing it, not by `isEnabled`: on iOS 18 the identifier
        // is on a container that is never marked disabled, though the button in it is, and drawn grey.)
        element("annotation-commit").tap()
        XCTAssertTrue(element("annotation-field").exists, "nothing to add before something is written")
        field.tap()
        field.typeText("Does it survive a restart?")
        XCTAssertTrue(element("annotation-commit").isEnabled)
        shot("41-notes-sheet")
        element("annotation-commit").tap()
        waitForTheSheetToGo()
        Thread.sleep(forTimeInterval: 1.0)
        shot("42-notes-drawn")

        // The word is a note now: its menu offers to change that one, not to add another.
        let edit = selectWord(word("Redis"), until: "Edit annotation")
        XCTAssertNotNil(edit, "a word with a note offers to edit it")
        XCTAssertFalse(entry("Add annotation").exists)
        edit?.tap()
        let again = requireElement("annotation-field", timeout: 10)
        XCTAssertEqual(again.value as? String, "Does it survive a restart?", "the note comes back as it was written")
        again.tap()
        again.typeText(" And a failover?")
        element("annotation-commit").tap()
        waitForTheSheetToGo()

        let changed = selectWord(word("Redis"), until: "Edit annotation")
        changed?.tap()
        XCTAssertEqual(requireElement("annotation-field", timeout: 10).value as? String, "Does it survive a restart? And a failover?")
        element("annotation-delete").tap()
        waitForTheSheetToGo()

        XCTAssertNotNil(selectWord(word("Redis"), until: "Add annotation"), "once the note is gone the word offers a new one")
        // Away without writing: nothing is kept.
        app.navigationBars.firstMatch.tap()
    }

    /// Giving the writing up leaves no note behind: at once when nothing is written, after a question when something is.
    func testCancellingTheSheetKeepsNothing() {
        openNotes()
        XCTAssertTrue(word("Memcached").waitForExistence(timeout: 60), "the page is drawn")

        selectWord(word("Memcached"), until: "Add annotation")?.tap()
        _ = requireElement("annotation-field", timeout: 10)
        element("annotation-cancel").tap()
        waitForTheSheetToGo()

        selectWord(word("Memcached"), until: "Add annotation")?.tap()
        requireElement("annotation-field", timeout: 10).typeText("never mind")
        element("annotation-cancel").tap()
        let discard = app.buttons["Discard"]
        XCTAssertTrue(discard.waitForExistence(timeout: 5), "words that would be lost are asked about")
        shot("45-notes-discard-asked")
        // Not discarding: the button that says so where the system shows one, else a tap outside (which is how a
        // question that comes up beside its button is put away).
        if app.buttons["Keep editing"].exists {
            app.buttons["Keep editing"].tap()
        } else {
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5)).tap()
        }
        let putAway = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: discard)
        XCTAssertEqual(XCTWaiter().wait(for: [putAway], timeout: 5), .completed, "the question is gone")
        XCTAssertEqual(element("annotation-field").value as? String, "never mind", "keeping on leaves the words as they were")
        element("annotation-cancel").tap()
        XCTAssertTrue(discard.waitForExistence(timeout: 5))
        discard.tap()
        waitForTheSheetToGo()
        Thread.sleep(forTimeInterval: 0.5)
        XCTAssertNotNil(selectWord(word("Memcached"), until: "Add annotation"), "the word is still without a note")
        app.navigationBars.firstMatch.tap()
    }

    /// Plain text, and a message from a Mac without notes, take none: the menu is the system's own.
    func testAMessageThatTakesNoNotesHasNoEntry() {
        openNotes("req-notes-off")
        XCTAssertTrue(word("Redis").waitForExistence(timeout: 60), "the page is drawn")
        let copy = selectWord(word("Redis"), until: "Copy")
        XCTAssertNotNil(copy, "the system's menu is up")
        XCTAssertFalse(entry("Add annotation").exists, "but not with ours")
        XCTAssertFalse(entry("Edit annotation").exists)
        shot("43-notes-off")
    }

    func testAMessageDrawnAsPlainTextHasNoEntry() {
        openNotes("req-notes-plain")
        let text = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "I compared two candidates")).firstMatch
        XCTAssertTrue(text.waitForExistence(timeout: 20), "the text is there")
        let copy = selectWord(text, until: "Copy")
        XCTAssertNotNil(copy, "the system's menu is up")
        XCTAssertFalse(entry("Add annotation").exists)
        shot("44-notes-plain")
    }
}
