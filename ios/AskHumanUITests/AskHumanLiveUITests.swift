import XCTest

/// The real app (no mock backend) on a simulator, paired with a throwaway Mac over the real broker
/// and relay. The Mac side is run by a host process (`SimulatorLiveTests` in the AskHumanKit E2E
/// target), which answers the rendezvous points below through files in `AH_LIVE_DIR`:
///
///     <dir>/ui/<name>     written here: "the UI is at <name>" (the content is a payload for the host)
///     <dir>/host/<name>   written by the host once it did its part (the content is a payload for us)
///
/// The pairing code arrives through a named pipe (`<dir>/code`) when the host is ready: it holds the
/// broker password, so it is never written to a file and no screenshot is taken while it is on
/// screen. Without `AH_LIVE_DIR` the test is skipped, which is what every ordinary run of this target
/// does.
@MainActor
final class AskHumanLiveUITests: XCTestCase {
    private var app: XCUIApplication!
    private var dir: URL!
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    private var shotCount = 0

    override func setUp() async throws {
        continueAfterFailure = false
        guard let path = ProcessInfo.processInfo.environment["AH_LIVE_DIR"], !path.isEmpty else {
            throw XCTSkip("AH_LIVE_DIR is not set: this test needs the host that runs the Mac side")
        }
        dir = URL(fileURLWithPath: path)
    }

    // MARK: the scenario

    func testPairingQuestionsAndPushOnTheRealApp() throws {
        // Markers for the host: when the run dies early they show how far it got.
        announce("started")
        launch()
        announce("launched")

        try step("pair with the Mac") {
            requireElement("pair-a-mac").tap()
            let field = requireElement("pairing-code")
            field.tap()
            // The Mac opens its (five-minute) pairing window only now and hands the code over a pipe.
            announce("pair-ready")
            field.typeText(try pairingCode())
            requireElement("pair-button").tap()
            let sas = requireElement("pairing-sas", timeout: 120).label
            shot("pairing-check-number")
            // The host compares it with the Mac's, then clicks Allow there.
            _ = rendezvous("sas", sas)
            XCTAssertTrue(
                app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Paired with'")).firstMatch.waitForExistence(timeout: 120),
                "the pairing did not finish"
            )
            shot("pairing-done")
        }

        try step("allow notifications") {
            let allow = springboard.alerts.buttons["Allow"]
            // SpringBoard can take a minute to show the alert on a busy machine; a tap can also get lost,
            // so ask again when the system stays quiet for long.
            for attempt in 1...2 {
                let button = app.buttons["Turn on notifications"]
                if button.waitForExistence(timeout: 10) { button.tap() }
                if allow.waitForExistence(timeout: 90) { break }
                shot("no-permission-alert-\(attempt)")
            }
            XCTAssertTrue(allow.exists, "the system asked nothing")
            allow.tap()
            XCTAssertTrue(app.tabBars.buttons["Inbox"].waitForExistence(timeout: 40), "the pairing sheet did not close")
            // The simulator hands out a device token; the phone registers it with the relay ("On" in
            // the Macs tab) and the Mac gets the handle (the host polls for it).
            app.tabBars.buttons["Macs"].tap()
            var status = requireElement("push-status", timeout: 40)
            let started = Date()
            var relaunches = 0
            // The row reads "Notifications, On" once the relay handed out a handle.
            while !status.label.hasSuffix("On") {
                Thread.sleep(forTimeInterval: 1)
                // A simulator that has just booted can sit on the first token request for minutes (its push
                // daemon starts late on a busy machine) and ignores asking again in the same process: what
                // works, as for a person, is to quit the app and open it again.
                if Date().timeIntervalSince(started) > Double(relaunches + 1) * 60 {
                    guard relaunches < 3 else { break }
                    relaunches += 1
                    app.terminate()
                    app.launch()
                    XCTAssertTrue(app.tabBars.buttons["Macs"].waitForExistence(timeout: 60), "the app did not come back")
                    app.tabBars.buttons["Macs"].tap()
                    status = requireElement("push-status", timeout: 60)
                }
            }
            let phoneSide = status.label
            shot("notifications-status")
            let state = rendezvous("push-registered", "phone=\(phoneSide) relaunches=\(relaunches) after=\(Int(Date().timeIntervalSince(started) * 1000))")
            XCTAssertEqual(state, "ok", "the Mac holds no push handle for this phone (the phone says: \(phoneSide))")
            app.tabBars.buttons["Inbox"].tap()
            shot("inbox-after-pairing")
        }

        try step("answer a question in the foreground") {
            let asked = Double(rendezvous("ask-single")) ?? 0
            let row = requireRow("inbox-row-", timeout: 90)
            let shownAt = nowMs()
            shot("single-in-inbox")
            row.tap()
            tapElement("option-0-1")
            shot("single-answer")
            sendForm()
            let sentAt = nowMs()
            XCTAssertTrue(rowGone("inbox-row-", timeout: 90), "the request stayed in the inbox")
            let leftAt = nowMs()
            // Both figures include the automation's own polling, so they are upper bounds.
            _ = rendezvous("ask-single-answered", "shown=\(shownAt - asked) left=\(leftAt - sentAt)")
        }

        try step("answer two questions with a free text") {
            _ = rendezvous("ask-multi")
            requireRow("inbox-row-", timeout: 90).tap()
            tapElement("option-0-0")
            reveal("answer-text-0")
            let note = requireElement("answer-text-0")
            note.tap()
            note.typeText("because")
            tapElement("option-1-1")
            shot("multi-answer")
            sendForm()
            XCTAssertTrue(rowGone("inbox-row-", timeout: 90), "the request stayed in the inbox")
            _ = rendezvous("ask-multi-answered")
        }

        try step("allow and refuse the permission prompts of an Agent") {
            // First one is allowed, the second refused with a reason; both come with the Mac's own choices.
            _ = rendezvous("permission-allow")
            requireRow("inbox-row-", timeout: 90).tap()
            let choices = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'choice-'"))
            XCTAssertTrue(choices.firstMatch.waitForExistence(timeout: 40), "the prompt offered no choices")
            XCTAssertGreaterThanOrEqual(choices.count, 2, "a permission prompt offers at least allow and refuse")
            let submit = requireElement("submit-button")
            XCTAssertFalse(submit.isEnabled, "nothing is chosen yet")
            shot("permission-prompt")
            choices.firstMatch.tap()
            XCTAssertTrue(submit.isEnabled, "the choice did not enable the button")
            submit.tap()
            XCTAssertTrue(rowGone("inbox-row-", timeout: 90), "the prompt stayed in the inbox")
            _ = rendezvous("permission-allow-done")

            _ = rendezvous("permission-deny")
            requireRow("inbox-row-", timeout: 90).tap()
            XCTAssertTrue(choices.firstMatch.waitForExistence(timeout: 40), "the second prompt offered no choices")
            choices.allElementsBoundByIndex.last?.tap()
            let reason = requireElement("confirm-input")
            reason.tap()
            reason.typeText("Run the tests first.")
            shot("permission-refuse")
            requireElement("submit-button").tap()
            XCTAssertTrue(rowGone("inbox-row-", timeout: 90), "the prompt stayed in the inbox")
            _ = rendezvous("permission-deny-done")
        }

        try step("watch and message an Agent that the Mac notifies about") {
            // The host reports a working session the way an Agent's own hooks do. The app has no follow
            // button: the launch option follows that session as soon as the Mac lists it.
            _ = rendezvous("agent-start")
            app.tabBars.buttons["Agents"].tap()
            let row = requireElement("agent-row-claude:live-agent", timeout: 60)
            shot("agents-list")
            row.tap()
            _ = requireElement("watch-card")
            // The live badge shows once the first frame came in from the Mac.
            _ = requireElement("watch-live", timeout: 60)
            shot("agent-watch")

            // The line only shows once the Mac has confirmed the follow.
            let note = requireElement("follow-note", timeout: 90)
            XCTAssertTrue(note.label.contains("finishes or fails"), "the page says: \(note.label)")
            XCTAssertFalse(element("follow-button").exists, "the app has no follow button any more")
            shot("agent-followed")

            // A message to the working Agent is queued until its next step.
            let field = requireElement("interject-field")
            field.tap()
            field.typeText("Also run the lint.")
            requireElement("interject-send").tap()
            _ = requireElement("interject-pending", timeout: 60)
            shot("interject-queued")
            // The host makes the Agent take that step; the read receipt is on screen for a few seconds only.
            announce("agent-message")
            XCTAssertTrue(waitForNotice("The Agent read your message.", timeout: 90), "no read receipt for the message")
            shot("interject-read")
            XCTAssertTrue(rowGone("interject-pending", timeout: 30), "the message stayed queued")

            // The step is the Agent's current tool in the list.
            app.navigationBars.buttons.firstMatch.tap()
            let tool = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS 'cargo test'")).firstMatch
            XCTAssertTrue(tool.waitForExistence(timeout: 60), "the list never showed the Agent's tool call")
            // A Mac the phone follows an Agent on marks it with the bell.
            XCTAssertTrue(requireElement("agent-row-claude:live-agent").label.contains("Will notify you"), "the list does not mark the Agent")
            shot("agents-list-working")
        }

        try step("see what a Mac that has Agent tasks switched off says") {
            requireElement("new-task").tap()
            let why = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS 'Agent tasks are turned off'")).firstMatch
            XCTAssertTrue(why.waitForExistence(timeout: 60), "the form did not say that tasks are switched off")
            shot("task-switched-off")
            requireElement("task-cancel").tap()
            XCTAssertTrue(rowGone("task-cancel", timeout: 20), "the form stayed open")
        }

        try step("get a push while the app is away") {
            // The host times how long the Mac takes to notice that the phone left.
            let asked = leaveForTheHomeScreen(announcing: "leaving", rendezvousAt: "away")
            // The banner stays for a few seconds only: whatever it says, it goes to the host, which knows
            // what it should say (the decrypted question, not the generic text).
            guard let push = openPushBanner(askedAt: asked) else {
                _ = rendezvous("pushed", "seen=-1 text=(no banner)")
                return XCTFail("no notification banner arrived")
            }
            _ = rendezvous("pushed", "seen=\(push.seen) text=\(push.text)")
            requireElement("option-0-0").tap()
            shot("push-opened-request")
            sendForm()
            XCTAssertTrue(rowGone("inbox-row-", timeout: 90), "the request stayed in the inbox")
            _ = rendezvous("pushed-answered")
        }

        // The Agent is still followed: nothing has finished yet (the first result is the one that counts).
        try step("get a push when a followed Agent finishes while the app is away") {
            let asked = leaveForTheHomeScreen(announcing: "leaving-again", rendezvousAt: "away-again")
            guard let push = openPushBanner(askedAt: asked) else {
                _ = rendezvous("follow-pushed", "seen=-1 text=(no banner)")
                return XCTFail("no notification banner arrived for the followed Agent")
            }
            _ = rendezvous("follow-pushed", "seen=\(push.seen) text=\(push.text)")
            // The tap on a follow notification opens the page of the Agent.
            _ = requireElement("watch-card", timeout: 60)
            shot("follow-push-opened")
        }

        try step("the answered requests are in the history") {
            // The page of the Agent covers the tab bar: go back to the list first.
            app.navigationBars.buttons.firstMatch.tap()
            app.tabBars.buttons["History"].tap()
            _ = requireRow("history-row-", timeout: 40)
            shot("history")
        }

        try step("unpair from the phone") {
            app.tabBars.buttons["Macs"].tap()
            requireRow("mac-row-", timeout: 40).tap()
            shot("mac-detail")
            reveal("Unpair this Mac", by: \.buttons)
            app.buttons["Unpair this Mac"].tap()
            let confirm = app.buttons["Unpair"]
            XCTAssertTrue(confirm.waitForExistence(timeout: 30), "no confirmation to unpair")
            confirm.tap()
            _ = rendezvous("unpaired")
            shot("after-unpair")
        }
    }

    // MARK: helpers

    private func launch() {
        let app = XCUIApplication()
        // The app follows the host's made-up Agent session as soon as it is listed (see `followAgent` in the app).
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-AHFollowAgent", "claude:live-agent"]
        app.launch()
        self.app = app
    }

    private func step(_ name: String, _ body: () throws -> Void) throws {
        try XCTContext.runActivity(named: name) { _ in try body() }
    }

    /// Reads the code from the named pipe `<AH_LIVE_DIR>/code`, which the host writes once it is ready
    /// (the code is never in a file or in the environment).
    private func pairingCode() throws -> String {
        let fd = open(dir.appendingPathComponent("code").path, O_RDONLY | O_NONBLOCK)
        XCTAssertGreaterThanOrEqual(fd, 0, "cannot open the pipe for the pairing code")
        defer { close(fd) }
        var data = Data()
        var chunk = [UInt8](repeating: 0, count: 4096)
        let deadline = Date().addingTimeInterval(180)
        while Date() < deadline {
            let count = read(fd, &chunk, chunk.count)
            if count > 0 {
                data.append(chunk, count: count)
                if data.last == 10 { break }
            } else {
                // 0: the host has not opened its end yet (or is done); -1: nothing to read yet.
                Thread.sleep(forTimeInterval: 0.1)
            }
        }
        let code = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        XCTAssertTrue(code.hasPrefix("ahpair1."), "the host did not hand over a pairing code")
        return code
    }

    private func nowMs() -> Double { (Date().timeIntervalSince1970 * 1000).rounded() }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func requireElement(_ id: String, timeout: TimeInterval = 40, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let found = element(id)
        XCTAssertTrue(found.waitForExistence(timeout: timeout), "\(id) did not appear", file: file, line: line)
        return found
    }

    /// The first element whose identifier starts with `prefix` (rows carry the id of their request).
    private func requireRow(_ prefix: String, timeout: TimeInterval, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let row = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", prefix)).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: timeout), "no \(prefix)… row appeared", file: file, line: line)
        return row
    }

    private func rowGone(_ prefix: String, timeout: TimeInterval) -> Bool {
        let row = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", prefix)).firstMatch
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: row)
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    private func tapElement(_ id: String) {
        reveal(id)
        requireElement(id).tap()
    }

    /// Sends the form. With several questions the bar offers "Next" until the last one has been on
    /// screen (as the Mac's popup does): this walks down with it, then presses "Send".
    private func sendForm() {
        var taps = 0
        while !element("send-button").exists, taps < 10 {
            let next = element("next-button")
            guard next.waitForExistence(timeout: 5) else { break }
            next.tap()
            taps += 1
            // The scroll to the next question animates, and the bar changes once that question is seen.
            Thread.sleep(forTimeInterval: 0.8)
        }
        requireElement("send-button").tap()
    }

    /// Waits until the element's accessibility label equals `label`.
    private func waitForLabel(_ id: String, _ label: String, timeout: TimeInterval = 30) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", label), object: element(id))
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    /// Waits for the short-lived notice of an Agent page (it is on screen for three seconds only, so
    /// this looks often instead of waiting on an expectation).
    private func waitForNotice(_ text: String, timeout: TimeInterval) -> Bool {
        let notice = element("interject-notice")
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if notice.exists, notice.label == text { return true }
            Thread.sleep(forTimeInterval: 0.2)
        }
        return false
    }

    /// Presses Home and waits for the host's go-ahead (it first waits until the Mac knows that the phone
    /// is away, then does what should reach the phone as a push). Returns when the host acted, in ms.
    private func leaveForTheHomeScreen(announcing leaving: String, rendezvousAt away: String) -> Double {
        announce(leaving)
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(
            app.wait(for: .runningBackground, timeout: 30) || app.state == .runningBackgroundSuspended,
            "the app did not leave the foreground"
        )
        return Double(rendezvous(away)) ?? 0
    }

    /// Waits for a notification banner, notes how long it took and what it says, and taps it: the banner
    /// goes away after a few seconds, so nothing else comes first. Opens the app.
    private func openPushBanner(askedAt: Double) -> (seen: Double, text: String)? {
        let banner = springboard.descendants(matching: .any).matching(identifier: "NotificationShortLookView").firstMatch
        guard banner.waitForExistence(timeout: 90) else {
            shot("push-missing")
            return nil
        }
        let seen = nowMs() - askedAt
        let text = banner.label
        shot("push-banner")
        banner.tap()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 40), "the tap did not open the app")
        return (seen, text)
    }

    /// Scrolls until the element exists and sits clear of the bars at either end.
    private func reveal(_ id: String, by kind: KeyPath<XCUIApplication, XCUIElementQuery>? = nil) {
        let target = kind.map { app[keyPath: $0][id] } ?? element(id)
        var tries = 0
        while !target.exists, tries < 8 {
            app.swipeUp(velocity: .slow)
            tries += 1
        }
        XCTAssertTrue(target.waitForExistence(timeout: 30), "\(id) did not appear")
        let screen = app.windows.firstMatch.frame
        for _ in 0..<10 {
            let frame = target.frame
            if frame.minY > 120, frame.maxY < screen.height - 220 { return }
            if frame.minY <= 120 { app.swipeDown(velocity: .slow) } else { app.swipeUp(velocity: .slow) }
        }
    }

    /// Tells the host that the UI is at `name` without waiting for anything.
    private func announce(_ name: String, _ payload: String = "") {
        try? payload.write(to: dir.appendingPathComponent("ui/\(name)"), atomically: true, encoding: .utf8)
    }

    /// Rendezvous with the host: announces `name` (and a payload), waits for the host's part, returns its payload.
    private func rendezvous(_ name: String, _ payload: String = "", timeout: TimeInterval = 300) -> String {
        let mine = dir.appendingPathComponent("ui/\(name)")
        let theirs = dir.appendingPathComponent("host/\(name)")
        try? payload.write(to: mine, atomically: true, encoding: .utf8)
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let reply = try? String(contentsOf: theirs, encoding: .utf8) { return reply }
            Thread.sleep(forTimeInterval: 0.1)
        }
        XCTFail("the host did not answer \(name)")
        return ""
    }

    /// Saves a screenshot into `AH_UI_SHOTS` (cheap to browse). Never call it while the pairing code is on screen.
    private func shot(_ name: String) {
        guard let folder = ProcessInfo.processInfo.environment["AH_UI_SHOTS"], !folder.isEmpty else { return }
        shotCount += 1
        let url = URL(fileURLWithPath: folder).appendingPathComponent(String(format: "live-%02d-%@.png", shotCount, name))
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }
}
