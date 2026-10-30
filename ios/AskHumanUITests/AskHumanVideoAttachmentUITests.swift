import XCTest

/// Seed an isolated simulator with the videos described in docs/plans/ios-video-attachments.md.
@MainActor final class AskHumanVideoAttachmentUITests: XCTestCase {
    private var app: XCUIApplication?

    override func setUpWithError() throws {
        continueAfterFailure = false
        guard ProcessInfo.processInfo.environment["AH_VIDEO_PICKER_QA"] == "YES" else {
            throw XCTSkip("Requires the isolated video-picker fixture library")
        }
    }

    override func tearDown() {
        app?.terminate()
        app = nil
        super.tearDown()
    }

    private func launch(agent: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-AHMock", "YES", "-AHScenario", "demo", "-AHWebMarkdown", "NO",
                               "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchArguments += agent ? ["-AHOpenAgent", "claude:cc-flaky"] : ["-AHOpen", "req-migration"]
        app.launch()
        self.app = app
        if !agent {
            let add = app.buttons["add-attachment-0"]
            for _ in 0..<10 where !add.exists { app.swipeUp(velocity: .slow) }
            XCTAssertTrue(add.waitForExistence(timeout: 10))
        }
        return app
    }

    private func select(_ duration: String, in app: XCUIApplication, agent: Bool = false, withPhoto: Bool = false) {
        let add = app.buttons[agent ? "interject-attach" : "add-attachment-0"]
        XCTAssertTrue(add.waitForExistence(timeout: 15))
        add.tap()
        app.buttons["Photo Library"].tap()
        let video = app.images.matching(NSPredicate(format: "label BEGINSWITH %@", "Video, \(duration),")).firstMatch
        XCTAssertTrue(video.waitForExistence(timeout: 45), "the seeded video must be selectable in the real photo picker")
        video.tap()
        if withPhoto {
            let photo = app.images.matching(NSPredicate(format: "label BEGINSWITH 'Photo,'")).firstMatch
            XCTAssertTrue(photo.waitForExistence(timeout: 10))
            photo.tap()
        }
        XCTAssertTrue(app.buttons["Done"].isEnabled)
        app.buttons["Done"].tap()
    }

    private func expectAttachments(_ count: Int, in app: XCUIApplication) {
        let remove = app.buttons.matching(NSPredicate(format: "label == 'Remove attachment'"))
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "count == %d", count), object: remove)
        XCTAssertEqual(XCTWaiter.wait(for: [expectation], timeout: 30), .completed)
        XCTAssertFalse(app.alerts["Could not attach"].exists)
    }

    func testVideoAttachmentFlows() {
        XCTContext.runActivity(named: "Show compression progress and let the user cancel") { activity in
            checkManualCancellation()
            record(activity)
        }
        XCTContext.runActivity(named: "Mix a photo with a small original video") { activity in
            checkPhotoAndSmallVideo()
            record(activity)
        }
        XCTContext.runActivity(named: "Compress an oversized video") { activity in
            checkCompressedVideo()
            record(activity)
        }
        XCTContext.runActivity(named: "Reject an obviously unfittable video") { activity in
            checkUnfittableVideo()
            record(activity)
        }
        XCTContext.runActivity(named: "Attach a video to an Agent interjection") { activity in
            checkInterjection()
            record(activity)
        }
        print("VIDEO_ATTACHMENT_UI_ASSERTIONS_COMPLETE")
    }

    private func record(_ activity: XCTActivity) {
        let screenshot = XCTAttachment(screenshot: app!.screenshot())
        screenshot.lifetime = .keepAlways
        activity.add(screenshot)
        app?.terminate()
        app = nil
    }

    private func checkPhotoAndSmallVideo() {
        let app = launch()
        select("two seconds", in: app, withPhoto: true)
        expectAttachments(2, in: app)
    }

    private func checkCompressedVideo() {
        let app = launch()
        select("six seconds", in: app)
        expectAttachments(1, in: app)
    }

    private func checkUnfittableVideo() {
        let app = launch()
        select("thirty minutes", in: app)
        let alert = app.alerts["Could not attach"]
        XCTAssertTrue(alert.waitForExistence(timeout: 30))
        XCTAssertTrue(alert.staticTexts["This video may exceed 10 MB after compression. Trim it before attaching."].exists)
        alert.buttons["OK"].tap()
        XCTAssertFalse(app.buttons.matching(NSPredicate(format: "label == 'Remove attachment'")).firstMatch.exists)
    }

    private func checkManualCancellation() {
        let app = launch(agent: true)
        let field = app.textFields["interject-field"]
        XCTAssertTrue(field.waitForExistence(timeout: 10))
        field.tap()
        field.typeText("Keep this draft")
        select("forty-five seconds", in: app, agent: true)
        let progress = app.descendants(matching: .any).matching(identifier: "attachment-compression-progress").firstMatch
        let hasProgress = progress.waitForExistence(timeout: 20)
        let screenshot = app.screenshot()
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        try? screenshot.pngRepresentation.write(to: documents.appendingPathComponent("video-compression-progress.png"))
        if !hasProgress { print(app.debugDescription) }
        XCTAssertTrue(hasProgress)
        let cancel = app.buttons["cancel-attachment-preparation"]
        XCTAssertTrue(cancel.exists)
        XCTAssertFalse(app.buttons["interject-send"].isEnabled)
        cancel.tap()
        let disappeared = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: cancel)
        XCTAssertEqual(XCTWaiter.wait(for: [disappeared], timeout: 15), .completed)
        XCTAssertFalse(app.alerts["Could not attach"].exists)
        XCTAssertFalse(app.buttons.matching(NSPredicate(format: "label == 'Remove attachment'")).firstMatch.exists)
        XCTAssertEqual(field.value as? String, "Keep this draft")
        XCTAssertTrue(app.buttons["interject-send"].isEnabled)
    }

    private func checkInterjection() {
        let app = launch(agent: true)
        select("two seconds", in: app, agent: true)
        expectAttachments(1, in: app)
        XCTAssertTrue(app.buttons["interject-send"].isEnabled)
    }
}
