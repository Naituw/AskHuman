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
        link.tap()
        let open = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "example.com")).firstMatch
        XCTAssertTrue(open.waitForExistence(timeout: 5), "the person is asked, with the domain, before the link opens")
        shot("32-link-asks")
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
}
