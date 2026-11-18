import XCTest

/// Photos picked together for a new task, through the real photo picker. The simulator's library must hold at least
/// six photos, as seeded with `xcrun simctl addmedia <udid> IMG_000*.jpg`; run with `TEST_RUNNER_AH_TASK_PHOTOS_QA=YES`.
@MainActor final class AskHumanTaskPhotosUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        guard ProcessInfo.processInfo.environment["AH_TASK_PHOTOS_QA"] == "YES" else {
            throw XCTSkip("Requires the isolated photo fixture library")
        }
        app = XCUIApplication()
        app.launchArguments = ["-AHMock", "YES", "-AHScenario", "demo", "-AHLatency", "0", "-AHWebMarkdown", "NO",
                               "-AHTab", "agents", "-AHNewTask", "YES", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
    }

    override func tearDown() {
        app?.terminate()
        app = nil
        super.tearDown()
    }

    /// The names of the rows on the screen, in order.
    private func visibleNames() -> [String] {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Photo' AND label ENDSWITH '.jpg'"))
            .allElementsBoundByIndex.map(\.label)
    }

    /// Every name the list holds, found by scrolling through it: a row below the fold is not on the screen and not in
    /// the accessibility tree either, so counting what is visible would miss it.
    private func allNames() -> [String] {
        var names: [String] = []
        func record() { for name in visibleNames() where !names.contains(name) { names.append(name) } }
        for _ in 0..<6 { app.swipeDown(velocity: .fast) }
        record()
        for _ in 0..<8 {
            app.swipeUp(velocity: .slow)
            record()
        }
        return names
    }

    private func reveal(_ element: XCUIElement) {
        for _ in 0..<12 where !(element.exists && element.isHittable) { app.swipeUp(velocity: .slow) }
        XCTAssertTrue(element.waitForExistence(timeout: 10))
    }

    /// Opens the picker and picks the first `count` photos it lists.
    private func pick(_ count: Int, file: StaticString = #filePath, line: UInt = #line) {
        let attach = app.buttons["task-attach"]
        reveal(attach)
        attach.tap()
        app.buttons["Photo Library"].tap()
        let photos = app.images.matching(NSPredicate(format: "label BEGINSWITH 'Photo,'"))
        XCTAssertTrue(photos.firstMatch.waitForExistence(timeout: 45), "the seeded photos must be listed", file: file, line: line)
        XCTAssertGreaterThanOrEqual(photos.count, count, "the library holds too few photos", file: file, line: line)
        for index in 0..<count { photos.element(boundBy: index).tap() }
        let done = app.buttons["Done"]
        if done.waitForExistence(timeout: 3) { done.tap() } else { app.buttons["Add"].tap() }
    }

    /// Waits until the list holds `expected` files, and returns their names.
    @discardableResult
    private func waitForFiles(_ expected: Int, timeout: TimeInterval = 40, file: StaticString = #filePath, line: UInt = #line) -> [String] {
        let started = Date()
        Thread.sleep(forTimeInterval: 6)
        var names = allNames()
        while names.count < expected, Date().timeIntervalSince(started) < timeout {
            Thread.sleep(forTimeInterval: 5)
            names = allNames()
        }
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "files-\(expected)"
        attachment.lifetime = .keepAlways
        add(attachment)
        print("TASK_PHOTOS files: \(names)")
        XCTAssertEqual(names.count, expected, "\(names.count) of \(expected) files were added: \(names)", file: file, line: line)
        XCTAssertFalse(app.alerts["Could not attach"].exists, file: file, line: line)
        return names
    }

    func testPhotosPickedTogetherAllArrive() {
        pick(4)
        waitForFiles(4)
    }

    func testAPhotoAddedLaterHasAName() {
        pick(2)
        waitForFiles(2)
        pick(1)
        let names = waitForFiles(3)
        XCTAssertEqual(Set(names).count, names.count, "every file has a name of its own: \(names)")
    }
}
