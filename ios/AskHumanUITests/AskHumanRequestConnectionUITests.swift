import XCTest

@MainActor
final class AskHumanRequestConnectionUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }
    private func localized(_ en: String, _ zh: String) -> String { language == "en" ? en : zh }

    override func setUp() async throws { continueAfterFailure = false }

    private func launch(request: String, scenario: String = "syncing") {
        app = XCUIApplication()
        app.launchArguments = ["-AHMock", "YES", "-AHScenario", scenario, "-AHLatency", "0.05",
                               "-AHCached", "YES", "-AHSyncDelay", "18", "-AHOpen", request,
                               "-AHWebMarkdown", "NO", "-AHAutoAdvance", "NO",
                               "-AppleLanguages", "(\(language))", "-AppleLocale", language == "en" ? "en_US" : "zh_CN"]
        app.launch()
        _ = require("request-connection")
    }

    private func element(_ id: String) -> XCUIElement { app.descendants(matching: .any).matching(identifier: id).firstMatch }
    @discardableResult
    private func require(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let item = element(id)
        XCTAssertTrue(item.waitForExistence(timeout: timeout), "missing \(id)", file: file, line: line)
        return item
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"] {
            let url = URL(fileURLWithPath: folder).appendingPathComponent("\(language)-\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
        } else {
            let attachment = XCTAttachment(screenshot: screenshot); attachment.lifetime = .keepAlways; add(attachment)
        }
    }

    private func waitUntilReady(_ button: XCUIElement) {
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: button)
        XCTAssertEqual(XCTWaiter().wait(for: [ready], timeout: 25), .completed)
        XCTAssertFalse(element("request-connection").exists)
    }

    func testCachedAnswerConnectsWithoutClaimingTheMacIsOffline() {
        launch(request: "req-select")
        XCTAssertEqual(require("request-connection").label, localized("Connecting…", "正在连接…"))
        require("option-0-0").tap()
        let send = require("send-button")
        XCTAssertFalse(send.isEnabled)
        shot("answer-connecting")
        waitUntilReady(send)
        XCTAssertTrue(element("option-0-0").isSelected)
        shot("answer-reconnected")
    }

    func testCachedConfirmationConnectsAndKeepsItsChoice() {
        launch(request: "req-permission")
        XCTAssertEqual(require("request-connection").label, localized("Connecting…", "正在连接…"))
        require("choice-0").tap()
        let submit = require("submit-button")
        XCTAssertFalse(submit.isEnabled)
        shot("confirm-connecting")
        waitUntilReady(submit)
        XCTAssertTrue(element("choice-0").isSelected)
        shot("confirm-reconnected")
    }

    func testInterruptedPhoneConnectionShowsRetryingOnBothForms() {
        for request in ["req-select", "req-permission"] {
            launch(request: request, scenario: "offline")
            XCTAssertEqual(require("request-connection").label, localized("Connection interrupted. Retrying…", "连接已中断，正在重试…"))
            let button = require(request == "req-select" ? "send-button" : "submit-button")
            XCTAssertFalse(button.isEnabled)
            shot(request == "req-select" ? "answer-retrying" : "confirm-retrying")
        }
    }
}
