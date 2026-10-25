import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The Swift phone engine against the real Rust gateway in a real daemon, over a local broker.
///
///   AH_E2E_BIN=<installed AskHuman> AH_E2E_BROKER_DIR=ios/Tools/local-broker swift test --filter AHE2ETests
final class E2ETests: E2ECase {
    func testPairingThenAnAnswerReachesTheAskingProcess() async throws {
        try await pair()
        let devices = try await mac.jsonArray(["channel", "ios", "devices", "--json"])
        XCTAssertEqual(devices.count, 1)
        XCTAssertEqual(devices.first?["name"] as? String, "E2E iPhone")

        let ask = try mac.ask(["-q", "Pick one", "-o", "Alpha", "-o", "Beta"])
        await eventually("the request on the phone") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        XCTAssertEqual(item.view?.ask?.questions.first?.text, "Pick one")
        XCTAssertEqual(item.view?.ask?.questions.first?.options.map(\.text), ["Alpha", "Beta"])

        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [1])]
        )
        let status = await ask.wait(timeout: 20)
        XCTAssertEqual(status, 0, ask.stderr)
        XCTAssertTrue(ask.stdout.contains("Beta"), "the answer text: \(ask.stdout)")
        XCTAssertFalse(ask.stdout.contains("Alpha"))
        await eventually("the request to leave the inbox") { await self.phone.state().inbox.isEmpty }
        let history = await phone.engine.history()
        XCTAssertEqual(history.first?.outcome, .answered)
    }

    // MARK: secrets

    /// The broker password, the pairing code and both direction keys must not show up in the daemon
    /// log or in anything the commands print (apart from the pairing code that `ios pair` shows on
    /// purpose), neither during pairing nor while a question is asked and answered.
    func testSecretsNeverReachTheLogsOrTheCommandOutput() async throws {
        let code = try await mac.startPairing()
        try await phone.pair(with: mac, code: code)

        let ask = try mac.ask(["-q", "Pick one", "-o", "Alpha", "-o", "Beta"])
        await eventually("the request on the phone") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        _ = try await phone.engine.submitAnswer(macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0])])
        await assertFinishes(ask)

        let status = try await mac.cli(["channel", "ios", "status", "--json"])
        let devices = try await mac.cli(["channel", "ios", "devices", "--json"])
        let list = try await mac.cli(["channel", "list"])
        let test = try await mac.cli(["channel", "test", "ios"])
        // A wrong password makes the broker refuse the connection: the error text must not repeat it.
        let wrong = try await mac.cli(["channel", "set", "ios", "--password-stdin"], stdin: "wrong-password-0123456789")
        try await Task.sleep(nanoseconds: 1_500_000_000)

        let keys = try XCTUnwrap(phone.vault.pairings().first)
        var secrets: [String: String] = ["the broker password": broker.password, "the pairing code": code]
        for (name, key) in [("the Mac-to-phone key", keys.m2p), ("the phone-to-Mac key", keys.p2m)] {
            secrets[name + " (base64)"] = key.base64EncodedString()
            secrets[name + " (base64url)"] = key.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
            secrets[name + " (hex)"] = key.map { String(format: "%02x", $0) }.joined()
        }
        secrets["the replaced password"] = "wrong-password-0123456789"

        var haystacks: [String: String] = [
            "ask stdout": ask.stdout, "ask stderr": ask.stderr, "status": status.stdout + status.stderr,
            "devices": devices.stdout + devices.stderr, "channel list": list.stdout + list.stderr,
            "channel test": test.stdout + test.stderr, "channel set": wrong.stdout + wrong.stderr,
        ]
        let files = FileManager.default.enumerator(at: mac.configDir, includingPropertiesForKeys: nil)?.compactMap { $0 as? URL } ?? []
        let logs = files.filter { $0.pathExtension == "log" }
        XCTAssertFalse(logs.isEmpty, "the daemon writes a log")
        for log in logs { haystacks[log.lastPathComponent] = (try? String(contentsOf: log, encoding: .utf8)) ?? "" }
        XCTAssertTrue(haystacks["daemon.log"]?.contains("ios") == true, "the log has content about the channel, so the scan means something")

        for (name, secret) in secrets {
            for (place, text) in haystacks where text.contains(secret) {
                XCTFail("\(name) leaked into \(place)")
            }
        }
        // The password lives in the secret store, not in the plain config file.
        let config = try String(contentsOf: mac.configDir.appendingPathComponent("config.json"), encoding: .utf8)
        XCTAssertFalse(config.contains(broker.password), "config.json holds no broker password")
    }

    // MARK: question shapes

    func testMultipleQuestionsWithFreeTextAndTheModeFlags() async throws {
        try await pair()
        let ask = try mac.ask([
            "Shared context for both", "--single",
            "-q", "First?", "-o", "Yes", "-o", "No",
            "-q", "Second?", "-o", "Red", "-o", "Blue",
        ])
        await eventually("the request") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        let view = try unwrap(item.view?.ask)
        XCTAssertEqual(view.message.text, "Shared context for both")
        XCTAssertEqual(view.questions.count, 2)
        XCTAssertTrue(view.single)
        XCTAssertFalse(view.selectOnly)

        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send",
            answers: [AnswerItem(q: 0, sel: [0], text: "because"), AnswerItem(q: 1, sel: [1])]
        )
        await assertFinishes(ask)
        XCTAssertTrue(ask.stdout.contains("Yes"), ask.stdout)
        XCTAssertTrue(ask.stdout.contains("because"), ask.stdout)
        XCTAssertTrue(ask.stdout.contains("Blue"), ask.stdout)
        XCTAssertTrue(ask.stdout.contains("# Q1") && ask.stdout.contains("# Q2"), ask.stdout)
    }

    func testTheMacRejectsAnswersThatBreakSelectOnly() async throws {
        try await pair()
        let ask = try mac.ask(["--select-only", "-q", "Pick", "-o", "A", "-o", "B"])
        await eventually("the request") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        XCTAssertEqual(item.view?.ask?.selectOnly, true)

        let bad = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [], text: "free text")]
        )
        await eventually("the rejection") {
            if case .failed = await self.phone.state().inbox.first?.state { return true }
            return false
        }
        XCTAssertTrue(ask.isRunning, "the asking process keeps waiting")
        await phone.engine.discardSubmission(clientMsgId: bad)

        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0])]
        )
        await assertFinishes(ask)
        XCTAssertTrue(ask.stdout.contains("A"), ask.stdout)
    }

    func testCancellingFromThePhoneEndsTheAskWithACancelledStatus() async throws {
        try await pair()
        let ask = try mac.ask(["-q", "Pick", "-o", "A"])
        await eventually("the request") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        _ = try await phone.engine.submitAnswer(macId: item.macId, requestId: item.requestId, action: "cancel", answers: [])
        _ = await ask.wait(timeout: 20)
        XCTAssertTrue(ask.stdout.contains("canceled"), ask.stdout)
        await eventually("history") { await self.phone.engine.history().first?.outcome == .cancelled }
    }

    func testTheCallerGivingUpEndsTheRequestOnThePhone() async throws {
        try await pair()
        let ask = try mac.ask(["-q", "Pick", "-o", "A"])
        await eventually("the request") { await self.phone.state().inbox.count == 1 }
        ask.interrupt()
        await eventually("the request to disappear") { await self.phone.state().inbox.isEmpty }
        let outcome = await phone.engine.history().first?.outcome
        XCTAssertTrue(outcome == .cancelled || outcome == .ended, "unexpected outcome \(String(describing: outcome))")
    }

    func testAMarkdownMessageArrivesIntact() async throws {
        try await pair()
        let message = "# Title\n\n- item `code`\n\n```swift\nlet x = 1\n```\n"
        let ask = try mac.ask(["--stdin", "-q", "OK?", "-o", "OK"], stdin: message)
        await eventually("the request") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        XCTAssertEqual(item.view?.ask?.isMarkdown, true)
        XCTAssertEqual(item.view?.ask?.message.text.trimmingCharacters(in: .whitespacesAndNewlines), message.trimmingCharacters(in: .whitespacesAndNewlines))
        ask.terminate()
    }

    // MARK: lifecycle

    func testARequestThatArrivedWhileTheAppWasInTheBackgroundIsFoundOnReturn() async throws {
        try await pair()
        await phone.engine.setForeground(false)
        await eventually("the phone to leave") {
            let phones = (try? await self.mac.jsonArray(["channel", "ios", "devices", "--json"])) ?? []
            return phones.first?["online"] as? Bool == false
        }
        let ask = try mac.ask(["-q", "While you were away", "-o", "A"])
        try await Task.sleep(nanoseconds: 1_500_000_000)
        let early = await phone.state().inbox.count
        XCTAssertEqual(early, 0)

        await phone.engine.setForeground(true)
        await eventually("the request after coming back") { await self.phone.state().inbox.count == 1 }
        ask.terminate()
    }

    func testRestartingTheMacDaemonReconnectsAndKeepsTheRequest() async throws {
        try await pair()
        let ask = try mac.ask(["-q", "Survive a restart?", "-o", "Yes"])
        await eventually("the request") { await self.phone.state().inbox.count == 1 }
        ask.terminate()
        _ = try await mac.cli(["daemon", "restart", "--force"], timeout: 60)
        await eventually("the Mac to come back", timeout: 60) { await self.phone.state().macs.first?.link == .online }
        await eventually("the first request to be gone") { await self.phone.state().inbox.isEmpty }
        let again = try mac.ask(["-q", "After the restart", "-o", "Yes"])
        await eventually("a new request") { await self.phone.state().inbox.count == 1 }
        again.terminate()
    }

    func testThePhoneRestartingKeepsItsPairingAndFindsThePendingRequestAgain() async throws {
        try await pair()
        let ask = try mac.ask(["-q", "Survive the app being killed?", "-o", "Yes", "-o", "No"])
        await eventually("the request") { await self.phone.state().inbox.count == 1 }
        let before = try unwrap(await phone.state().inbox.first)

        // The app is killed: its engine is gone, the keychain and the files stay. (The old object
        // stays alive until the end, or its deinit would delete the files the new one reads.)
        let old = try XCTUnwrap(phone)
        await old.engine.setForeground(false)
        await eventually("the phone to leave") {
            let phones = (try? await self.mac.jsonArray(["channel", "ios", "devices", "--json"])) ?? []
            return phones.first?["online"] as? Bool == false
        }
        phone = E2EPhone(secrets: old.secrets, root: old.root)
        await phone.engine.start()
        await phone.engine.setForeground(true)

        await eventually("the pairing to be back online") { await self.phone.state().macs.first?.link == .online }
        await eventually("the pending request to come back") { await self.phone.state().inbox.map(\.requestId) == [before.requestId] }
        _ = try await phone.engine.submitAnswer(
            macId: before.macId, requestId: before.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [1])]
        )
        await assertFinishes(ask)
        XCTAssertTrue(ask.stdout.contains("No"), ask.stdout)
        withExtendedLifetime(old) {}
    }

    func testUnpairingFromTheMacRemovesThePhonesSide() async throws {
        try await pair()
        let devices = try await mac.jsonArray(["channel", "ios", "devices", "--json"])
        let id = try unwrap(devices.first?["phoneId"] as? String ?? devices.first?["id"] as? String)
        let result = try await mac.cli(["channel", "ios", "revoke", id])
        XCTAssertEqual(result.status, 0, result.stderr)
        await eventually("the Mac to disappear from the phone") { await self.phone.state().macs.isEmpty }
        XCTAssertTrue(phone.vault.pairings().isEmpty)
    }

    func testUnpairingFromThePhoneRemovesTheDeviceOnTheMac() async throws {
        try await pair()
        let macId = try unwrap(await phone.state().macs.first?.id)
        await phone.engine.unpair(macId: macId)
        await eventually("the Mac to forget the phone") {
            let devices = (try? await self.mac.jsonArray(["channel", "ios", "devices", "--json"])) ?? []
            return devices.isEmpty
        }
    }

    // MARK: other request kinds

    private static let permissionPayload = """
    {"hook_event_name":"PermissionRequest","session_id":"e2e-session","cwd":"/tmp/e2e-project","permission_mode":"default","tool_name":"Bash","tool_input":{"command":"rm -rf build","description":"Remove the build folder"}}
    """

    func testAPermissionPromptIsDecidedOnThePhone() async throws {
        try await pair()
        let hook = try mac.ask(["__permission-hook", "claude"], stdin: Self.permissionPayload)
        await eventually("the confirmation") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        XCTAssertEqual(item.view?.kind, "confirm")
        let confirm = try unwrap(item.view?.confirm)
        XCTAssertTrue(confirm.context.contains { $0.value.contains("rm -rf build") || $0.value == "Bash" }, "context: \(confirm.context)")
        XCTAssertGreaterThanOrEqual(confirm.choices.count, 2)
        let allow = try unwrap(confirm.choices.first { $0.role == "primary" } ?? confirm.choices.first { $0.recommended })

        _ = try await phone.engine.submitConfirm(
            macId: item.macId, requestId: item.requestId, wireIndex: allow.wireIndex, dismiss: false, input: nil
        )
        await assertFinishes(hook)
        XCTAssertTrue(hook.stdout.contains("\"allow\""), "hook output: \(hook.stdout)")
        let entry = try unwrap(await phone.engine.history().first)
        XCTAssertEqual(entry.outcome, .answered)
        XCTAssertEqual(entry.confirmChoice, allow.label)
    }

    func testDenyingAPermissionPromptWithAReason() async throws {
        try await pair()
        let hook = try mac.ask(["__permission-hook", "claude"], stdin: Self.permissionPayload)
        await eventually("the confirmation") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        let confirm = try unwrap(item.view?.confirm)
        let deny = try unwrap(confirm.choices.first { $0.role == "danger" } ?? confirm.choices.last)
        _ = try await phone.engine.submitConfirm(
            macId: item.macId, requestId: item.requestId, wireIndex: deny.wireIndex, dismiss: false, input: "too risky"
        )
        await assertFinishes(hook)
        XCTAssertTrue(hook.stdout.contains("deny"), "hook output: \(hook.stdout)")
    }

    func testTheWhatsNextRequestIsFlaggedAndCarriesItsSuggestions() async throws {
        try await pair()
        let ask = try mac.ask(["--whats-next", "Finished the work", "-o", "Write the docs", "-o!", "Ship it"])
        await eventually("the request") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        let view = try unwrap(item.view?.ask)
        XCTAssertTrue(view.whatsNext)
        let options = view.questions.first?.options ?? []
        XCTAssertTrue(options.contains { $0.text == "Write the docs" })
        XCTAssertTrue(options.contains { $0.text == "Ship it" && $0.recommended })
        ask.terminate()
    }

    // MARK: several phones and Macs

    func testTwoPhonesRaceAndTheSlowerOneIsToldItLost() async throws {
        try await pair()
        let second = E2EPhone(deviceName: "Second iPhone")
        try await second.pair(with: mac, code: try await mac.startPairing())
        addTeardownBlock { await second.close() }

        let ask = try mac.ask(["-q", "Who is first?", "-o", "Me"])
        await eventually("both phones to get the request") {
            let first = await self.phone.state().inbox.count
            let other = await second.state().inbox.count
            return first == 1 && other == 1
        }
        let item = try unwrap(await phone.state().inbox.first)
        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0])]
        )
        await assertFinishes(ask)
        await eventually("the other phone to hear it lost") { await second.state().inbox.isEmpty }
        let outcome = await second.engine.history().first?.outcome
        XCTAssertEqual(outcome, .answeredElsewhere)
    }

    func testOnePhoneAnswersTwoMacsThroughTheSameBroker() async throws {
        try await pair()
        let otherMac = try await MacInstance(environment: environment, broker: broker, macName: "Office Mac")
        addTeardownBlock { await otherMac.stop() }
        try await phone.engine.finishPairing()
        await phone.engine.startPairing(code: try await otherMac.startPairing())
        await eventually("the second Mac to ask for confirmation") {
            let pairing = (try? await otherMac.status())?["pairing"] as? [String: Any]
            return pairing?["stage"] as? String == "pending"
        }
        _ = try await otherMac.cli(["channel", "ios", "allow"])
        await eventually("two Macs on the phone") {
            if case .done = await self.phone.state().pairing?.stage { return true }
            return false
        }
        await phone.engine.finishPairing()
        await eventually("both Macs online") {
            let macs = await self.phone.state().macs
            return macs.count == 2 && macs.allSatisfy { $0.link == .online }
        }

        let homeAsk = try mac.ask(["-q", "From home?", "-o", "Yes"])
        let officeAsk = try otherMac.ask(["-q", "From the office?", "-o", "Yes"])
        await eventually("both requests") { await self.phone.state().inbox.count == 2 }
        let inbox = await phone.state().inbox
        XCTAssertEqual(Set(inbox.map(\.macName)), ["E2E Mac", "Office Mac"])
        for item in inbox {
            _ = try await phone.engine.submitAnswer(
                macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0])]
            )
        }
        await assertFinishes(homeAsk)
        await assertFinishes(officeAsk)
    }

    // MARK: attachments

    func testAnAttachedFileIsDownloadedFromTheMac() async throws {
        try await pair()
        let file = mac.home.appendingPathComponent("notes.txt")
        let body = String(repeating: "line of text\n", count: 40_000)
        try body.write(to: file, atomically: true, encoding: .utf8)
        let ask = try mac.ask(["See the file", "-f", file.path, "-q", "Fine?", "-o", "Yes"])
        await eventually("the request") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        let asset = try unwrap(item.view?.ask?.message.files.first)
        XCTAssertEqual(asset.name, "notes.txt")
        XCTAssertEqual(asset.size, UInt64(body.utf8.count))
        let seen = ProgressLog()
        let url = try await phone.engine.download(macId: item.macId, asset: asset) { seen.record($0) }
        XCTAssertEqual(try String(contentsOf: url, encoding: .utf8), body)
        XCTAssertEqual(seen.last, 1)
        ask.terminate()
    }

    func testAReplyAttachmentArrivesAsAFileOnTheMac() async throws {
        try await pair()
        let ask = try mac.ask(["-q", "Send me a file", "-o", "Done"])
        await eventually("the request") { await self.phone.state().inbox.count == 1 }
        let item = try unwrap(await phone.state().inbox.first)
        let payload = Data((0..<400_000).map { UInt8($0 % 251) })
        let uploadId = try await phone.engine.upload(
            macId: item.macId, requestId: item.requestId,
            source: UploadSource(name: "blob.bin", mime: "application/octet-stream", data: payload, isImage: false)
        ) { _ in }
        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send", answers: [AnswerItem(q: 0, sel: [0], files: [uploadId])]
        )
        await assertFinishes(ask)
        let line = try unwrap(ask.stdout.split(separator: "\n").map(String.init).first { $0.contains("blob.bin") })
        let path = line.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "- ", with: "")
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: path)), payload)
    }
}

final class ProgressLog: @unchecked Sendable {
    private let lock = NSLock()
    private var all: [Double] = []
    func record(_ value: Double) { lock.withLock { all.append(value) } }
    var last: Double? { lock.withLock { all.last } }
}
