import XCTest

/// A host-side launcher activates the separate receiver while XCTest keeps the drag finger down.
@MainActor
final class AskHumanAttachmentDragUITests: XCTestCase {
    func testAnUnopenedPictureCanBeDraggedToAnotherApp() throws {
        try drag(picture: true, preview: false)
    }

    func testAnUnopenedFileCanBeDraggedToAnotherApp() throws {
        try drag(picture: false, preview: false)
    }

    func testAnOpenedPictureCanBeDraggedToAnotherApp() throws {
        try drag(picture: true, preview: true)
    }

    func testAnOpenedFileCanBeDraggedToAnotherApp() throws {
        try drag(picture: false, preview: true)
    }

    private func drag(picture: Bool, preview: Bool) throws {
        continueAfterFailure = false
        guard ProcessInfo.processInfo.environment["AH_DRAG_RECEIVER"] == "1" else {
            throw XCTSkip("Run with the host-side receiver launcher in ios/Tools/drag-receiver.")
        }
        let receiver = XCUIApplication(bundleIdentifier: "com.naituw.askhuman.drag-receiver")
        receiver.terminate()
        let app = XCUIApplication()
        app.launchArguments = ["-AHMock", "YES", "-AHScenario", "gallery",
                               "-AHLatency", "0.1", "-AHWebMarkdown", "NO", "-AHAutoAdvance", "NO", "-AHDeveloperMode", "NO",
                               "-AHDragReceiverTest", "YES",
                               "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
        let row = app.buttons["inbox-row-req-gallery"]
        XCTAssertTrue(row.waitForExistence(timeout: 20))
        row.tap()
        let query = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", picture ? "asset-picture-" : "asset-file-"))
        let source = query.firstMatch
        XCTAssertTrue(source.waitForExistence(timeout: 20))
        if !picture, source.frame.maxY > app.frame.maxY - 100 { app.swipeUp() }
        XCTAssertTrue(source.isHittable)
        var frame = source.frame
        if preview {
            source.tap()
            if picture {
                XCTAssertTrue(app.descendants(matching: .any)["image-gallery"].waitForExistence(timeout: 20))
                Thread.sleep(forTimeInterval: 2)
                frame = CGRect(x: app.frame.midX, y: app.frame.height * 0.4, width: 0, height: 0)
            } else {
                let content = app.descendants(matching: .any)["asset-file-preview"]
                XCTAssertTrue(content.waitForExistence(timeout: 20))
                frame = content.frame
            }
        }
        let start = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: frame.midX, dy: frame.midY))
        start.press(forDuration: 1.5, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.65)),
                    withVelocity: .slow, thenHoldForDuration: 6)
        let result = receiver.staticTexts["drop-result"]
        XCTAssertTrue(result.waitForExistence(timeout: 15))
        let received = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label BEGINSWITH 'Received '"), object: result)
        XCTAssertEqual(XCTWaiter().wait(for: [received], timeout: 20), .completed, result.label)
        XCTAssertTrue(result.label.contains(picture ? ".png" : ".md"), result.label)
        XCTAssertTrue(picture ? result.label.contains("public.png") : (result.label.contains("text") || result.label.contains("markdown")), result.label)
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
