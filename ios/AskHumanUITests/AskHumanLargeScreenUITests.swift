import XCTest
import UIKit

/// The inbox in a window wide enough for two columns: an iPad, or the inner screen of a foldable phone.
/// A phone-sized window keeps the stack, which the other navigation tests cover, so these skip there.
@MainActor
final class AskHumanLargeScreenUITests: XCTestCase {
    private var app: XCUIApplication!
    private var language: String { ProcessInfo.processInfo.environment["AH_UI_LANG"] ?? "en" }

    override func setUp() async throws { continueAfterFailure = false }

    /// The two columns side by side. Narrower (a foldable's inner screen held upright) the list lies over
    /// the request and is put away once a request was picked.
    private var isTiled: Bool { app.windows.firstMatch.frame.width >= 700 }

    private func launch(request: String? = nil, scenario: String = "demo", autoAdvance: Bool = false,
                        extra: [String] = []) throws {
        let app = XCUIApplication()
        var arguments = ["-AHMock", "YES", "-AHLatency", "0.05", "-AHScenario", scenario,
                         "-AHWebMarkdown", "NO", "-AHAutoAdvance", autoAdvance ? "YES" : "NO",
                         "-AppleLanguages", "(\(language))", "-AppleLocale", language == "en" ? "en_US" : "zh_CN"]
        if let request { arguments += ["-AHOpen", request] }
        app.launchArguments = arguments + extra
        app.launch()
        self.app = app
        try XCTSkipIf(app.windows.firstMatch.frame.width < 600, "A phone-sized window keeps the stack.")
    }

    /// Keeps what the screen shows: with the test results, and in the folder named by `AH_UI_SHOTS` when
    /// there is one (a person looking at the layout, not a check).
    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"] {
            try? FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name).png"))
        }
    }

    private func element(_ id: String) -> XCUIElement { app.descendants(matching: .any).matching(identifier: id).firstMatch }

    @discardableResult
    private func require(_ id: String, timeout: TimeInterval = 20, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let item = element(id)
        XCTAssertTrue(item.waitForExistence(timeout: timeout), "missing \(id)", file: file, line: line)
        return item
    }

    private func title(_ expected: String, file: StaticString = #filePath, line: UInt = #line) {
        let label = require("request-title", file: file, line: line)
        let wait = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", expected), object: label)
        XCTAssertEqual(XCTWaiter().wait(for: [wait], timeout: 12), .completed, file: file, line: line)
    }

    private func becomes(_ item: XCUIElement, _ predicate: String, timeout: TimeInterval = 12,
                         file: StaticString = #filePath, line: UInt = #line) {
        let wait = XCTNSPredicateExpectation(predicate: NSPredicate(format: predicate), object: item)
        XCTAssertEqual(XCTWaiter().wait(for: [wait], timeout: timeout), .completed, "\(predicate)", file: file, line: line)
    }

    func testTheInboxStartsOnTheListWithNothingPicked() throws {
        try launch()
        let row = require("inbox-row-req-migration")
        XCTAssertTrue(row.isHittable, "the list is what there is to pick from")
        XCTAssertFalse(element("request-title").exists, "no request is open before one is picked")
        if isTiled { require("detail-placeholder") }
    }

    func testPickingARowOpensItAndTheArrowsMoveTheSelection() throws {
        try launch()
        require("inbox-row-req-migration").tap()
        title("Plan the users table migration")
        if isTiled {
            let first = element("inbox-row-req-migration")
            XCTAssertTrue(first.isHittable, "the list stays beside the request")
            becomes(first, "isSelected == true")
            require("next-request").tap()
            title("收尾 iOS 通知与分组的细节")
            becomes(element("inbox-row-req-next"), "isSelected == true")
            becomes(first, "isSelected == false")
        } else {
            becomes(element("inbox-row-req-migration"), "isHittable == false", timeout: 20)
            require("next-request")
        }
    }

    func testARequestOpenedFromOutsideTheListIsTheOneTheListShows() throws {
        try launch(request: "req-migration")
        title("Plan the users table migration")
        if isTiled {
            becomes(element("inbox-row-req-migration"), "isSelected == true")
            XCTAssertFalse(element("detail-placeholder").exists)
        }
    }

    /// The button of the bar that calls the list back: the one at the left end of the bar over the request.
    /// It is the system's, with a label of its own in every language, so it is told by where it is.
    private func sidebarToggle() -> XCUIElement {
        let buttons = app.navigationBars.buttons.allElementsBoundByIndex.filter { $0.isHittable }
        return buttons.min { $0.frame.minX < $1.frame.minX } ?? app.navigationBars.buttons.firstMatch
    }

    /// Where the list lies over the request (the inner screen of a foldable held upright): it is put away
    /// once a request was picked, called back with the button of the bar, and put away by the next pick.
    func testTheListCalledBackOverARequestIsPutAwayByAnotherPick() throws {
        try launch()
        try XCTSkipIf(isTiled, "Beside each other, the list is never over the request.")
        let first = element("inbox-row-req-migration"), second = element("inbox-row-req-next")
        require("inbox-row-req-migration").tap()
        title("Plan the users table migration")
        becomes(first, "isHittable == false", timeout: 20)
        shot("narrow-1-put-away")

        let toggle = sidebarToggle()
        if !toggle.waitForExistence(timeout: 20) { add(XCTAttachment(string: app.debugDescription)) }
        XCTAssertTrue(toggle.exists, "the bar has the button that calls the list back")
        toggle.tap()
        becomes(second, "isHittable == true", timeout: 20)
        shot("narrow-2-called-back")

        second.tap()
        title("收尾 iOS 通知与分组的细节")
        becomes(second, "isHittable == false", timeout: 20)
        shot("narrow-3-second-pick")
    }

    /// The button of the list's own bar that puts the list away, for a list beside the request: the one
    /// at the right end of that bar (the bar's own filter button is at the left end).
    private func listToggleBesideTheRequest() -> XCUIElement {
        let listEdge = element("inbox-row-req-migration").frame.maxX
        let buttons = app.navigationBars.buttons.allElementsBoundByIndex.filter { $0.isHittable && $0.frame.midX < listEdge }
        return buttons.max { $0.frame.maxX < $1.frame.maxX } ?? app.navigationBars.buttons.firstMatch
    }

    /// Like `shot`, from the app's window: `XCUIScreen.main` is the cover screen of a foldable, which is
    /// black while the app is on the inner one.
    private func windowShot(_ name: String) {
        let screenshot = app.windows.firstMatch.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        if let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"] {
            try? FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name).png"))
        }
    }

    /// The share of a row's pixels that are tinted: the blue of the row that is open, where every other row
    /// is a neutral grey (or white) card. Told by the pixels, because what the row says (`isSelected`) is
    /// what the tint used to disagree with.
    private func tintedShare(of row: XCUIElement) -> Double {
        let window = app.windows.firstMatch
        let screenshot = window.screenshot().image
        guard let image = screenshot.cgImage else { return 0 }
        let scale = screenshot.scale, frame = row.frame.offsetBy(dx: -window.frame.minX, dy: -window.frame.minY)
        let area = CGRect(x: frame.minX * scale, y: frame.minY * scale, width: frame.width * scale, height: frame.height * scale)
            .integral.intersection(CGRect(x: 0, y: 0, width: image.width, height: image.height))
        guard !area.isEmpty, let crop = image.cropping(to: area) else { return 0 }
        let width = crop.width, height = crop.height
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        guard let context = CGContext(
            data: &pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return 0 }
        context.draw(crop, in: CGRect(x: 0, y: 0, width: width, height: height))
        var tinted = 0
        for index in stride(from: 0, to: pixels.count, by: 4) {
            let red = Int(pixels[index]), green = Int(pixels[index + 1]), blue = Int(pixels[index + 2])
            if blue >= red + 25, blue >= green + 8 { tinted += 1 }
        }
        return Double(tinted) / Double(width * height)
    }

    /// The pick that moved while the list was out of sight. The arrows of the request that is open move
    /// it, and so do a notification and a banner, with the list put away (over the request, or with the
    /// button of its bar). Whatever the page that is open is, the row of it has the tint when the list is
    /// back, and the row that was open has the card the others have again: the row that was tinted used
    /// to come back with neither, so that no row looked picked and, with two rows, the other looked it.
    func testThePickThatMovedWhileTheListWasOutOfSightIsTintedWhenTheListIsBack() throws {
        addTeardownBlock { XCUIDevice.shared.appearance = .light }
        for (style, appearance) in [("Light", XCUIDevice.Appearance.light), ("Dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            try launch()
            let first = element("inbox-row-req-migration"), second = element("inbox-row-req-next")
            first.tap()
            title("Plan the users table migration")
            if isTiled {
                listToggleBesideTheRequest().tap()
            }
            becomes(first, "isHittable == false", timeout: 20)

            require("next-request").tap()
            title("收尾 iOS 通知与分组的细节")
            // Beside the request the next pick brings the list back by itself; over it, the button does.
            if isTiled {
                becomes(second, "isHittable == true", timeout: 20)
            } else {
                sidebarToggle().tap()
                becomes(second, "isHittable == true", timeout: 20)
            }
            // (The list may still be sliding in: the tint is looked for until it is there or time is up.)
            var tinted = false
            let deadline = Date().addingTimeInterval(10)
            while !tinted, Date() < deadline {
                tinted = tintedShare(of: second) > 0.3
                if !tinted { RunLoop.current.run(until: Date().addingTimeInterval(0.4)) }
            }
            windowShot("picked-while-away-\(style.lowercased())")
            XCTAssertTrue(tinted, "the row of the request that is open is tinted (\(style))")
            XCTAssertLessThan(tintedShare(of: first), 0.05, "the row that was open is a card like the others (\(style))")
            XCTAssertLessThan(tintedShare(of: element("inbox-row-req-select")), 0.05, "so is a row that never was (\(style))")
            app.terminate()
        }
    }

    func testReselectingInboxRevealsTheFloatingListAndKeepsTheDraft() throws {
        try launch(request: "req-select")
        try XCTSkipIf(isTiled, "The sidebar only floats in a narrower split view.")
        let option = require("option-0-0")
        option.tap()
        XCTAssertTrue(option.isSelected)

        let row = element("inbox-row-req-select")
        becomes(row, "isHittable == false")
        let inbox = app.tabBars.buttons[language == "en" ? "Inbox" : "收件箱"]
        XCTAssertTrue(inbox.isSelected)
        inbox.tap()
        becomes(row, "isHittable == true")
        XCTAssertTrue(row.isSelected, "the open request stays selected")

        app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5)).tap()
        becomes(row, "isHittable == false")
        XCTAssertTrue(option.isSelected, "the draft survives showing and dismissing the list")
    }

    /// Sending the one request that is open, with nothing to follow it, is what going back to the list is
    /// on a phone. Side by side that is the column of the request becoming the placeholder again; where
    /// the list lies over the request it is the list that is called back.
    func testSendingWithNothingToFollowLeadsBackToTheList() throws {
        // The receipt stays long enough to be seen by a test that is slow to ask for it.
        try launch(request: "req-select", extra: ["-AHAdvanceHold", "5"])
        require("option-0-0").tap()
        require("send-button").tap()
        _ = require("sent-receipt", timeout: 10)
        becomes(element("sent-receipt"), "exists == false", timeout: 20)
        let row = require("inbox-row-req-migration")
        becomes(row, "isHittable == true", timeout: 20)
        XCTAssertFalse(element("inbox-row-req-select").exists, "what was sent no longer waits in the list")
        XCTAssertFalse(element("request-title").exists, "no request is open any more")
        if isTiled {
            require("detail-placeholder")
            XCTAssertFalse(row.isSelected)
        }
        shot("sent-back-to-the-list")
    }

    /// With the next request waiting, sending leads to it: it takes the place of the one that was sent,
    /// the list picks it, and a list lying over the request stays away.
    func testSendingLeadsToTheNextRequestAndTheListFollows() throws {
        try launch(request: "req-select", autoAdvance: true, extra: ["-AHAdvanceHold", "2", "-AHAdvanceSettle", "1.2"])
        require("option-0-0").tap()
        require("send-button").tap()
        _ = require("sent-receipt", timeout: 10)
        _ = require("sent-pill", timeout: 10)
        becomes(element("sent-receipt"), "exists == false", timeout: 20)
        let next = require("submit-button", timeout: 10)
        XCTAssertFalse(element("inbox-row-req-select").exists, "what was sent no longer waits in the list")
        shot("sent-next")
        if isTiled {
            // (A row can show up as more than one element, each with the row's identifier.)
            func pickedRows() -> Set<String> {
                Set(app.descendants(matching: .any)
                    .matching(NSPredicate(format: "identifier BEGINSWITH 'inbox-row-' AND isSelected == true"))
                    .allElementsBoundByIndex.map(\.identifier))
            }
            let settled = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in pickedRows().count == 1 }, object: nil)
            XCTAssertEqual(XCTWaiter().wait(for: [settled], timeout: 10), .completed,
                           "the list picks the request that took its place: \(pickedRows())")
        }
        becomes(next, "isHittable == true", timeout: 20)
    }

    /// An iPad turns in all four ways: the columns stay, and so does what is open.
    func testTurningAnIPadKeepsTheOpenRequestAndTheList() throws {
        try launch()
        try XCTSkipIf(UIDevice.current.userInterfaceIdiom != .pad, "A foldable is turned by folding and by its own screen.")
        defer { XCUIDevice.shared.orientation = .portrait }
        shot("ipad-portrait-list")
        require("inbox-row-req-migration").tap()
        title("Plan the users table migration")
        shot("ipad-portrait-open")
        for (name, orientation) in [("landscape-left", UIDeviceOrientation.landscapeLeft), ("portrait", .portrait),
                                    ("landscape-right", .landscapeRight), ("upside-down", .portraitUpsideDown),
                                    ("portrait-again", .portrait)] {
            XCUIDevice.shared.orientation = orientation
            title("Plan the users table migration")
            becomes(element("inbox-row-req-migration"), "isSelected == true")
            XCTAssertTrue(element("inbox-row-req-migration").isHittable, "the list stays beside the request in \(name)")
            shot("ipad-\(name)-open")
        }
        XCUIDevice.shared.orientation = .landscapeLeft
        require("next-request").tap()
        title("收尾 iOS 通知与分组的细节")
        shot("ipad-landscape-next")
    }

    /// Before a Mac is paired there is the introduction and nothing to pick from: in columns all the same,
    /// so that pairing the first Mac does not change the page under the bar an iPad has at its top.
    func testWithoutAMacTheIntroductionHasTheWindow() throws {
        try launch(scenario: "empty")
        require("pair-a-mac")
        XCTAssertFalse(element("inbox-row-req-migration").exists)
        shot("no-mac")
    }

    func testWithoutAMacTheAgentsAndTheTodosHaveTheIntroductionToo() throws {
        for tab in ["agents", "todos"] {
            try launch(scenario: "empty", extra: ["-AHTab", tab])
            require("pair-a-mac")
            XCTAssertFalse(element("detail-placeholder").exists, "nothing to pick from before a Mac is paired (\(tab))")
            shot("no-mac-\(tab)")
            app.terminate()
        }
    }

    // MARK: the accessibility sizes

    /// At the accessibility sizes of text a list column holds hardly a word of a row, so a wide window
    /// keeps the phone's stack: the list has the window, a pick is pushed over it, and back is the list.
    func testAccessibilityTextKeepsThePhonesStackInAWideWindow() throws {
        try launch(extra: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityL"])
        let row = require("inbox-row-req-migration")
        XCTAssertTrue(row.isHittable)
        XCTAssertFalse(element("detail-placeholder").exists, "there is no right column at these sizes")
        // The widest the column of a list ever is: a row wider than that is not in one.
        XCTAssertGreaterThan(row.frame.width, 400, "the list has the window")
        shot("ax-list")
        row.tap()
        title("Plan the users table migration")
        becomes(row, "isHittable == false", timeout: 20)
        shot("ax-pushed")
        // Duo keeps the stack's back button in its floating bar, outside navigationBars.
        let back = element("BackButton")
        if back.exists {
            XCTAssertTrue(back.isHittable)
            back.tap()
        } else {
            sidebarToggle().tap()
        }
        becomes(element("inbox-row-req-migration"), "isHittable == true", timeout: 20)
    }

    // MARK: the other tabs

    private let workingAgent = "claude:cc-flaky", waitingAgent = "cursor:cur-migrate"
    private let homeMac = "mockhomemacmacmacmacmacmac"

    /// What each tab opens, and what its page says before anything was picked, is the same story as the
    /// inbox's: the list first, a pick opens the page and tints the row, another pick moves the tint.
    func testTheAgentsTabOpensASessionBesideTheList() throws {
        try launch(extra: ["-AHTab", "agents"])
        let working = require("agent-row-\(workingAgent)"), waiting = require("agent-row-\(waitingAgent)")
        XCTAssertTrue(working.isHittable, "the list is what there is to pick from")
        XCTAssertFalse(element("watch-card").exists, "no session is open before one is picked")
        if isTiled { require("detail-placeholder") }
        shot("agents-list")
        working.tap()
        _ = require("watch-card")
        if isTiled {
            XCTAssertTrue(working.isHittable, "the list stays beside the session")
            becomes(working, "isSelected == true")
            waiting.tap()
            becomes(waiting, "isSelected == true")
            becomes(working, "isSelected == false")
            _ = require("watch-card")
        } else {
            becomes(working, "isHittable == false", timeout: 20)
        }
        shot("agents-open")
    }

    func testASessionOpenedFromOutsideIsTheOneTheListShows() throws {
        try launch(extra: ["-AHTab", "agents", "-AHOpenAgent", waitingAgent])
        _ = require("watch-card")
        if isTiled {
            becomes(element("agent-row-\(waitingAgent)"), "isSelected == true")
            XCTAssertFalse(element("detail-placeholder").exists)
        }
    }

    func testTheTodosTabOpensAProjectBesideTheList() throws {
        try launch(extra: ["-AHTab", "todos"])
        let project = require("todo-project-a1b2c3d4e5f60001")
        XCTAssertTrue(project.isHittable, "the list is what there is to pick from")
        XCTAssertFalse(element("todo-project-screen").exists, "no project is open before one is picked")
        if isTiled { require("detail-placeholder") }
        shot("todos-list")
        project.tap()
        _ = require("todo-project-screen")
        if isTiled {
            XCTAssertTrue(project.isHittable, "the list stays beside the project")
            becomes(project, "isSelected == true")
            let billing = require("todo-project-a1b2c3d4e5f60002")
            billing.tap()
            becomes(billing, "isSelected == true")
            becomes(project, "isSelected == false")
            _ = require("todo-project-screen")
        } else {
            becomes(project, "isHittable == false", timeout: 20)
        }
        shot("todos-open")
    }

    func testTheHistoryOpensAnEntryBesideTheList() throws {
        try launch(extra: ["-AHTab", "history"])
        let titled = require("history-row-req-old-1"), plain = require("history-row-req-old-2")
        XCTAssertTrue(titled.isHittable, "the list is what there is to pick from")
        XCTAssertFalse(element("history-title").exists, "no entry is open before one is picked")
        if isTiled { require("detail-placeholder") }
        shot("history-list")
        titled.tap()
        _ = require("history-title")
        if isTiled {
            XCTAssertTrue(titled.isHittable, "the list stays beside the entry")
            becomes(titled, "isSelected == true")
            plain.tap()
            becomes(plain, "isSelected == true")
            becomes(titled, "isSelected == false")
            becomes(element("history-title"), "exists == false")
        } else {
            becomes(titled, "isHittable == false", timeout: 20)
        }
        shot("history-open")
    }

    func testTheMacsTabOpensAMacBesideTheList() throws {
        try launch(extra: ["-AHTab", "macs"])
        let home = require("mac-row-\(homeMac)")
        XCTAssertTrue(home.isHittable, "the list is what there is to pick from")
        if isTiled { require("detail-placeholder") }
        shot("macs-list")
        home.tap()
        let page = app.navigationBars["Home MacBook Pro"]
        XCTAssertTrue(page.waitForExistence(timeout: 20), "the page of the Mac")
        if isTiled {
            XCTAssertTrue(home.isHittable, "the list stays beside the Mac")
            becomes(home, "isSelected == true")
        } else {
            becomes(home, "isHittable == false", timeout: 20)
        }
        shot("macs-open")
    }

    // MARK: the forms over the tabs

    /// What opens over the tabs is the system's sheet (a form sheet on an iPad): it has to fit the window and
    /// be usable there. These look at them; the checks are only that they open.
    func testTheFormToStartATaskOpensOverTheAgents() throws {
        try launch(extra: ["-AHTab", "agents", "-AHNewTask", "YES"])
        require("task-start", timeout: 30)
        shot("form-new-task")
        require("task-cancel").tap()
        becomes(element("task-start"), "exists == false")
        require("agent-row-\(workingAgent)")
    }

    func testTheFormToEditATodoOpensOverTheProject() throws {
        try launch(extra: ["-AHTab", "todos"])
        require("todo-project-a1b2c3d4e5f60001").tap()
        _ = require("todo-project-screen")
        let todo = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'todo-row-'")).firstMatch
        XCTAssertTrue(todo.waitForExistence(timeout: 20), "a todo to edit")
        todo.tap()
        require("todo-edit-field")
        shot("form-edit-todo")
        require("todo-edit-save")
    }

    func testThePairingFormOpensFromTheIntroduction() throws {
        try launch(scenario: "empty")
        require("pair-a-mac").tap()
        require("pairing-code")
        shot("form-pairing")
    }

    /// There is nothing to go back to beside the list: a Mac that was unpaired leaves the placeholder, and
    /// where the list lies over the page it is the list that comes back.
    func testUnpairingTheMacThatIsOpenLeadsBackToTheList() throws {
        try launch(extra: ["-AHTab", "macs"])
        require("mac-row-\(homeMac)").tap()
        let unpair = app.buttons[language == "en" ? "Unpair this Mac" : "解除与这台 Mac 的配对"]
        XCTAssertTrue(unpair.waitForExistence(timeout: 20))
        for _ in 0..<4 where !unpair.isHittable { app.swipeUp() }
        unpair.tap()
        app.buttons[language == "en" ? "Unpair" : "解除配对"].firstMatch.tap()
        becomes(element("mac-row-\(homeMac)"), "exists == false", timeout: 20)
        let office = require("mac-row-mockofficemacmacmacmacmac")
        becomes(office, "isHittable == true", timeout: 20)
        if isTiled { require("detail-placeholder") }
        shot("macs-unpaired")
    }
}
