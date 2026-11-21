import XCTest

/// Viewing a picture the Mac sends a preview of (`docs/specs/ios-attachment-traffic.md` §7.2), through the real UI over the
/// mock Macs' `previews` scenario, in English or Chinese (`TEST_RUNNER_AH_UI_LANG=zh-Hans`).
///
/// The request `req-previews` has five pictures: `wide-big.png` and `tall-big.png` (previews), `small.png` (too small for one),
/// `undecodable-preview.png` (a preview the phone cannot open) and `flaky-original.png` (the Mac fails to send its file the first
/// time it is asked for it). `-AHOriginalGate <path>` holds the files of the pictures that have previews back until the file
/// at that path exists, so a test can look at a download while it is under way.
@MainActor
final class AskHumanAttachmentPreviewUITests: XCTestCase {
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
    private func launch(
        scenario: String = "previews", open request: String? = "req-previews", extra: [String] = [], contentSize: String? = nil,
        dark: Bool = false
    ) -> XCUIApplication {
        app?.terminate()
        let app = XCUIApplication()
        let locale = language == "en" ? "en_US" : "zh_CN"
        var arguments = [
            "-AHMock", "YES", "-AHLatency", "0.05", "-AHScenario", scenario, "-AHWebMarkdown", "NO",
            "-AHAutoAdvance", "NO", "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
        ] + extra
        if let request { arguments += ["-AHOpen", request] }
        if let contentSize { arguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategory" + contentSize] }
        if dark {
            XCUIDevice.shared.appearance = .dark
            addTeardownBlock { XCUIDevice.shared.appearance = .light }
        }
        app.launchArguments = arguments
        app.launch()
        self.app = app
        if request != nil { _ = requireElement("request-title") }
        return app
    }

    private func localized(_ en: String, _ zh: String) -> String { language == "en" ? en : zh }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    @discardableResult
    private func requireElement(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let found = element(id)
        let exists = found.waitForExistence(timeout: timeout)
        if !exists { add(XCTAttachment(string: app.debugDescription)) }
        XCTAssertTrue(exists, "\(id) did not appear", file: file, line: line)
        return found
    }

    private func waitFor(_ message: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line, _ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return }
            Thread.sleep(forTimeInterval: 0.1)
        }
        XCTFail(message, file: file, line: line)
    }

    private func settle() { Thread.sleep(forTimeInterval: 1.0) }

    private func assertSameFrame(
        _ frame: CGRect, _ expected: CGRect, _ message: String, file: StaticString = #filePath, line: UInt = #line
    ) {
        let text = "\(message): \(frame) against \(expected)"
        XCTAssertEqual(frame.minX, expected.minX, accuracy: 1, text, file: file, line: line)
        XCTAssertEqual(frame.minY, expected.minY, accuracy: 1, text, file: file, line: line)
        XCTAssertEqual(frame.width, expected.width, accuracy: 1, text, file: file, line: line)
        XCTAssertEqual(frame.height, expected.height, accuracy: 1, text, file: file, line: line)
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"], !folder.isEmpty {
            let url = URL(fileURLWithPath: folder).appendingPathComponent("preview-\(language)-\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
            return
        }
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// A path the mock Mac waits for before it sends the file of a picture that has a preview. The test makes it exist
    /// (`openTheGate`) when it has seen what it wanted to of the download.
    private func holdTheFiles(_ name: String = #function) -> String {
        let path = "/tmp/ah-original-gate-" + name.filter { $0.isLetter }
        try? FileManager.default.removeItem(atPath: path)
        addTeardownBlock { try? FileManager.default.removeItem(atPath: path) }
        return path
    }

    private func openTheGate(_ path: String) {
        FileManager.default.createFile(atPath: path, contents: Data())
    }

    private func thumbnail(_ name: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label == %@", name)).firstMatch
    }

    /// Opens the viewer on a picture of the request and waits for the page to be there.
    private func openPicture(_ name: String, scrolling: Bool = false, file: StaticString = #filePath, line: UInt = #line) {
        let button = thumbnail(name)
        _ = button.waitForExistence(timeout: 8)
        if scrolling { bringOnScreen(button, file: file, line: line) }
        XCTAssertTrue(button.waitForExistence(timeout: 20), "\(name) has no thumbnail", file: file, line: line)
        button.tap()
        requireElement("image-gallery")
        waitFor("the viewer did not open on \(name)", file: file, line: line) { self.element("gallery-name").label == name }
    }

    /// At a large text size the pictures are further down the page than the screen reaches, and the answer panel covers the
    /// lower half of it: drags the page (above the panel) until the button is between the bar and the panel.
    private func bringOnScreen(_ button: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        let height = app.windows.firstMatch.frame.height
        /// Drags the page up by `distance` (down when it is negative) with a finger that starts above the panel.
        func drag(_ distance: CGFloat) {
            let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4))
            let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4 - distance / height))
            start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.1)
        }
        let limit = height * 0.3
        for _ in 0..<20 {
            guard button.exists else {
                drag(limit)
                continue
            }
            // The button's top is to rest at about a quarter of the screen: below the bar, well above the panel.
            let off = button.frame.minY - height * 0.27
            if abs(off) < height * 0.1 { return }
            drag(max(-limit, min(limit, off)))
        }
        XCTFail("\(button) could not be brought between the bar and the answer panel", file: file, line: line)
    }

    private func closeViewer() {
        requireElement("gallery-close").tap()
        waitFor("the viewer did not close") { !self.element("image-gallery").exists }
    }

    /// The system's share sheet, which a viewer opens over itself (an iPad shows it as a popover).
    private var shareSheet: XCUIElement { app.otherElements["ActivityListView"] }

    private func dismissShareSheet() {
        let close = app.navigationBars["UIActivityContentView"].buttons["Close"]
        if close.exists {
            close.tap()
        } else {
            // A card that hangs from the bar, a popover and a sheet from the bottom all close on a tap outside of them: on the
            // viewer's picture, on the side where the sheet is not.
            let window = app.windows.firstMatch
            let atTop = shareSheet.frame.midY < window.frame.midY
            window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: atTop ? 0.96 : 0.25)).tap()
        }
        waitFor("the share sheet did not close") { !self.shareSheet.exists }
    }

    // MARK: the preview first, the original on request

    func testAPictureWithAPreviewComesAsThatAndOffersTheOriginal() {
        launch()
        openPicture("wide-big.png")
        let original = requireElement("gallery-original")
        XCTAssertTrue(original.label.contains(localized("Original", "原图")), original.label)
        XCTAssertTrue(original.label.contains("3.4"), "it says how big the original is: \(original.label)")
        XCTAssertTrue(original.label.contains("MB"), original.label)
        XCTAssertTrue(element("gallery-share").exists, "the picture can be shared as soon as there is something to show")
        requireElement("viewer-picture-asset-prev-wide")
        settle()
        // The bar has room for the button and for the heading both (a bar that is too narrow drops the heading altogether).
        XCTAssertTrue(element("gallery-name").exists, "the heading is gone from the bar")
        XCTAssertGreaterThan(element("gallery-name").frame.width, 30, "the heading has no room in the bar")
        XCTAssertTrue(element("gallery-position").exists, "which picture it is is gone from the bar")
        shot("viewer-preview")
        closeViewer()
    }

    func testTheOriginalComesInPlaceOfThePreviewAndKeepsTheZoom() {
        let gate = holdTheFiles()
        launch(extra: ["-AHOriginalGate", gate])
        openPicture("wide-big.png")
        let picture = requireElement("viewer-picture-asset-prev-wide")
        let area = requireElement("viewer-zoom-asset-prev-wide")
        waitFor("the picture is not in place") { picture.frame.height > 100 }
        let rest = picture.frame
        area.doubleTap()
        waitFor("the picture did not zoom in") { picture.frame.height > rest.height * 2 }
        settle()
        let zoomed = picture.frame

        requireElement("gallery-original").tap()
        requireElement("gallery-original-progress")
        XCTAssertFalse(element("gallery-original").exists, "the button gives way to the progress while the file comes")
        XCTAssertEqual(element("gallery-original-progress").value as? String, "0%", "nothing has come yet")
        XCTAssertFalse(element("gallery-share").isEnabled, "sharing waits for the file that is on its way")
        settle()
        assertSameFrame(picture.frame, zoomed, "the preview stays where it is while the file comes")
        shot("viewer-original-progress")

        openTheGate(gate)
        waitFor("the progress did not end") { !self.element("gallery-original-progress").exists }
        XCTAssertFalse(element("gallery-original").exists, "the button is gone once the original is on the phone")
        settle()
        assertSameFrame(picture.frame, zoomed, "the original took the place of the preview, at the same zoom")
        XCTAssertTrue(element("gallery-share").isEnabled)
        shot("viewer-original-in-place")
        closeViewer()
    }

    func testThePageNextToTheOneOnShowComesAsAPreviewToo() {
        launch()
        openPicture("wide-big.png")
        requireElement("gallery-original")
        // The next page was fetched while the first was on show: its preview, not the file.
        element("image-gallery").swipeLeft()
        waitFor("the viewer did not page to the next picture") { self.element("gallery-name").label == "tall-big.png" }
        let original = requireElement("gallery-original")
        XCTAssertTrue(original.label.contains("2.1"), "the button of the next picture says how big its original is: \(original.label)")
        shot("viewer-next-page")
        closeViewer()
    }

    func testAPictureTooSmallForAPreviewHasNoOriginalButton() {
        launch()
        openPicture("small.png")
        requireElement("viewer-picture-asset-prev-small")
        settle()
        XCTAssertFalse(element("gallery-original").exists)
        XCTAssertFalse(element("gallery-original-progress").exists)
        XCTAssertTrue(element("gallery-share").exists)
        shot("viewer-no-preview")
        closeViewer()
    }

    // MARK: sharing

    func testSharingBeforeTheOriginalIsOnThePhoneFetchesItFirst() {
        let gate = holdTheFiles()
        launch(extra: ["-AHOriginalGate", gate])
        openPicture("wide-big.png")
        requireElement("gallery-original")
        requireElement("gallery-share").tap()
        requireElement("gallery-original-progress")
        XCTAssertFalse(element("gallery-original").exists)
        XCTAssertFalse(element("gallery-share").isEnabled, "a second tap would only ask again")
        // Nothing is shared while only the preview is on the phone.
        Thread.sleep(forTimeInterval: 1.5)
        XCTAssertFalse(shareSheet.exists, "the share sheet opened before the file came")
        shot("share-fetching")

        openTheGate(gate)
        waitFor("the share sheet did not open once the file had come", timeout: 30) { self.shareSheet.exists }
        shot("share-sheet")
        dismissShareSheet()

        XCTAssertFalse(element("gallery-original").exists, "the file is on the phone now")
        XCTAssertFalse(element("gallery-original-progress").exists)
        // The file is here: sharing is the system's own button again, and it opens the sheet at once.
        requireElement("gallery-share").tap()
        waitFor("the share sheet did not open at once", timeout: 10) { self.shareSheet.exists }
        dismissShareSheet()
        closeViewer()
    }

    func testSharingAPictureThatHasNoPreviewOpensTheShareSheetAtOnce() {
        launch()
        openPicture("small.png")
        requireElement("viewer-picture-asset-prev-small")
        requireElement("gallery-share").tap()
        waitFor("the share sheet did not open", timeout: 10) { self.shareSheet.exists }
        XCTAssertFalse(element("gallery-original-progress").exists, "there was nothing to fetch")
        dismissShareSheet()
        closeViewer()
    }

    func testSwipingAwayWhileTheFileIsFetchedForSharingOpensNoShareSheet() {
        let gate = holdTheFiles()
        launch(extra: ["-AHOriginalGate", gate])
        openPicture("wide-big.png")
        requireElement("gallery-share").tap()
        requireElement("gallery-original-progress")
        element("image-gallery").swipeLeft()
        waitFor("the viewer did not page to the next picture") { self.element("gallery-name").label == "tall-big.png" }
        openTheGate(gate)
        Thread.sleep(forTimeInterval: 3)
        XCTAssertFalse(shareSheet.exists, "a picture that is not on show was not meant to be shared")
        XCTAssertTrue(element("image-gallery").exists)
        element("image-gallery").swipeRight()
        waitFor("the viewer did not page back") { self.element("gallery-name").label == "wide-big.png" }
        XCTAssertFalse(element("gallery-original").exists, "the file did come: nothing more to ask for")
        closeViewer()
    }

    // MARK: when something goes wrong

    func testAPreviewThePhoneCannotOpenIsReplacedByTheOriginalWithoutAnError() {
        launch()
        openPicture("undecodable-preview.png")
        requireElement("viewer-picture-asset-prev-broken")
        settle()
        XCTAssertFalse(app.staticTexts[localized("Not a readable image", "无法读取该图片")].exists)
        XCTAssertFalse(app.staticTexts[localized("Could not open the file", "无法打开文件")].exists)
        XCTAssertFalse(app.alerts.firstMatch.exists)
        XCTAssertFalse(element("gallery-original").exists, "what is shown is the original already")
        XCTAssertTrue(element("gallery-share").isEnabled)
        shot("viewer-undecodable-preview")
        closeViewer()
    }

    func testAFailedOriginalIsReportedAndTheButtonAsksAgain() {
        launch()
        openPicture("flaky-original.png")
        requireElement("viewer-picture-asset-prev-flaky")
        requireElement("gallery-original").tap()
        let alert = app.alerts.firstMatch
        XCTAssertTrue(alert.waitForExistence(timeout: 20), "the failure was not reported")
        XCTAssertTrue(alert.staticTexts[localized("Could not download the original", "无法下载原图")].exists)
        XCTAssertTrue(alert.staticTexts[localized("The Mac did not answer in time.", "Mac 未及时响应。")].exists)
        XCTAssertTrue(element("viewer-picture-asset-prev-flaky").exists, "the preview stays")
        shot("original-failed")
        alert.buttons[localized("Cancel", "取消")].tap()
        waitFor("the alert stayed") { !self.app.alerts.firstMatch.exists }

        // The button is there to ask again, and the Mac answers this time.
        requireElement("gallery-original").tap()
        waitFor("the original did not come") { !self.element("gallery-original").exists && !self.element("gallery-original-progress").exists }
        XCTAssertFalse(app.alerts.firstMatch.exists)
        closeViewer()
    }

    func testTryAgainAfterAFailedOriginalFetchesItAndTheAlertGoesAway() {
        launch()
        openPicture("flaky-original.png")
        requireElement("gallery-original").tap()
        let alert = app.alerts.firstMatch
        XCTAssertTrue(alert.waitForExistence(timeout: 20))
        alert.buttons[localized("Try again", "重试")].tap()
        waitFor("the original did not come") { !self.element("gallery-original").exists && !self.element("gallery-original-progress").exists }
        XCTAssertFalse(app.alerts.firstMatch.exists)
        XCTAssertTrue(element("viewer-picture-asset-prev-flaky").exists)
        closeViewer()
    }

    func testAFailedFetchForSharingTriesAgainAndThenShares() {
        launch()
        openPicture("flaky-original.png")
        requireElement("viewer-picture-asset-prev-flaky")
        requireElement("gallery-share").tap()
        let alert = app.alerts.firstMatch
        XCTAssertTrue(alert.waitForExistence(timeout: 20), "the failure was not reported")
        XCTAssertFalse(shareSheet.exists, "there was nothing to share")
        alert.buttons[localized("Try again", "重试")].tap()
        waitFor("the share sheet did not open after the second try", timeout: 30) { self.shareSheet.exists }
        dismissShareSheet()
        closeViewer()
    }

    // MARK: the history

    /// The pictures in the history go through the same viewer (AT17): `onboarding-7.png` is the one the Mac makes a preview of.
    func testAPictureInTheHistoryOffersTheOriginalToo() {
        launch(scenario: "historyPictures", open: nil, extra: ["-AHTab", "history"])
        requireElement("history-row-ex-pics-05").tap()
        let seventh = requireElement("history-picture-m-grid-6")
        seventh.tap()
        requireElement("image-gallery")
        waitFor("the viewer did not open on the seventh picture") { self.element("gallery-name").label == "onboarding-7.png" }
        let original = requireElement("gallery-original")
        XCTAssertTrue(original.label.contains(localized("Original", "原图")), original.label)
        shot("history-viewer-preview")
        original.tap()
        waitFor("the original did not come") { !self.element("gallery-original").exists && !self.element("gallery-original-progress").exists }
        closeViewer()
    }

    // MARK: text size, appearance

    func testTheBarHoldsTheOriginalButtonAtTheLargestTextSize() {
        launch(contentSize: "AccessibilityXXXL")
        openPicture("wide-big.png", scrolling: true)
        let original = requireElement("gallery-original")
        let share = requireElement("gallery-share")
        let close = requireElement("gallery-close")
        let name = requireElement("gallery-name")
        let window = app.windows.firstMatch.frame
        for (what, frame) in [("original", original.frame), ("share", share.frame), ("close", close.frame), ("heading", name.frame)] {
            XCTAssertTrue(window.contains(frame), "the \(what) \(frame) is inside the screen \(window)")
        }
        XCTAssertFalse(original.frame.intersects(share.frame), "the buttons do not overlap")
        XCTAssertFalse(original.frame.intersects(close.frame), "the buttons do not overlap")
        XCTAssertFalse(name.frame.intersects(original.frame), "the heading does not run under the button")
        XCTAssertFalse(name.frame.intersects(close.frame), "the heading does not run under the close button")
        XCTAssertTrue(original.label.contains("MB"), "the size is still read out: \(original.label)")
        shot("viewer-large-text")
        closeViewer()
    }

    func testTheViewerInTheDarkLooksRight() {
        let gate = holdTheFiles()
        launch(extra: ["-AHOriginalGate", gate], dark: true)
        openPicture("wide-big.png")
        requireElement("gallery-original")
        settle()
        shot("viewer-dark")
        requireElement("gallery-original").tap()
        requireElement("gallery-original-progress")
        settle()
        shot("viewer-dark-progress")
        openTheGate(gate)
        waitFor("the progress did not end") { !self.element("gallery-original-progress").exists }
        closeViewer()
    }
}
