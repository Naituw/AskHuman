import XCTest

/// The real app (the real `PhoneEngine`, the keychain, the notification extension) on an iOS
/// simulator, against a throwaway Mac over the real broker and relay. The Mac side runs here; the
/// phone side is `AskHumanLiveUITests`, started through `xcodebuild`. The two meet at named
/// rendezvous points through files in a temporary folder.
///
/// Build the app and the UI tests once:
///
///   xcodebuild build-for-testing -project ios/AskHuman.xcodeproj -scheme AskHuman \
///     -destination 'platform=iOS Simulator,id=<udid>' -derivedDataPath ios/build/DerivedDataLive
///
/// then, from `ios/Packages/AskHumanKit`:
///
///   AH_E2E_BIN=<installed AskHuman> AH_E2E_REAL_FROM=<ASKHUMAN_HOME of a configured Mac> \
///   AH_E2E_BUNDLE_ID=<the app's bundle id> AH_E2E_SIM_UDID=<udid> swift test --filter SimulatorLiveTests
///
/// The simulator gets a clean start (the app is uninstalled, which also drops its notification
/// permission, and the device keychain is **reset**), so give it a device of its own. Erasing it
/// instead (`AH_E2E_SIM_ERASE=1`) works too but sets off the first-boot work of iOS, minutes of CPU
/// that starve the run; either way the run waits until the device is quiet before it starts.
/// `AH_E2E_SHOTS=<folder>` keeps the screenshots (default `/tmp/ah-live-shots`).
///
/// The pairing code holds the broker password: it goes to the UI test through a named pipe only (no
/// file ever holds it, and the Mac's five-minute pairing window starts when the UI test is ready for
/// the code, however long `xcodebuild` took to get there), the result bundle (whose automatic
/// screenshots may show it being typed) is deleted, and the run ends by scanning what it left behind
/// for the code.
final class SimulatorLiveTests: E2ECase {
    override func setUp() async throws {
        let env = ProcessInfo.processInfo.environment
        guard try E2EEnvironment.current().usesRealServices, env["AH_E2E_SIM_UDID"]?.isEmpty == false else {
            throw XCTSkip("AH_E2E_REAL_FROM and AH_E2E_SIM_UDID are needed: this drives the real app on a simulator")
        }
        try await super.setUp()
    }

    func testPairingQuestionsAndPushOnTheRealApp() async throws {
        let udid = try XCTUnwrap(ProcessInfo.processInfo.environment["AH_E2E_SIM_UDID"])
        let ios = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let derived = ios.appendingPathComponent("build/DerivedDataLive")
        try XCTSkipUnless(FileManager.default.fileExists(atPath: derived.path), "build the app first (see the doc comment)")
        try await resetSimulator(udid)

        let meeting = try Meeting()
        defer { meeting.remove() }
        let shots = ProcessInfo.processInfo.environment["AH_E2E_SHOTS"] ?? "/tmp/ah-live-shots"
        try? FileManager.default.removeItem(atPath: shots)
        let result = FileManager.default.temporaryDirectory.appendingPathComponent("ah-live-\(UUID().uuidString.prefix(8)).xcresult")

        var env = ProcessInfo.processInfo.environment
        env["TEST_RUNNER_AH_LIVE_DIR"] = meeting.dir.path
        env["TEST_RUNNER_AH_UI_SHOTS"] = shots
        let runner = Cmd(
            "/usr/bin/xcrun",
            [
                "xcodebuild", "test-without-building", "-project", "AskHuman.xcodeproj", "-scheme", "AskHuman",
                "-destination", "platform=iOS Simulator,id=\(udid)", "-derivedDataPath", derived.path,
                "-only-testing:AskHumanUITests/AskHumanLiveUITests", "-parallel-testing-enabled", "NO",
                "-collect-test-diagnostics", "never", "-resultBundlePath", result.path,
            ],
            env: env, cwd: ios
        )
        try runner.start()
        let live = Live(meeting: meeting, runner: runner)
        var numbers: [String: Any] = [:]
        var code = ""

        do {
            // The first wait covers xcodebuild starting up and installing the app: minutes on a busy Mac.
            // Only then does the Mac open its pairing window, and the code goes through the pipe.
            _ = try await live.arrive("pair-ready", timeout: 900)
            code = try await mac.startPairing()
            try await meeting.hand(code: code)

            // Pairing: both sides must show the same check number before the Mac allows.
            let phoneSas = try await live.arrive("sas", timeout: 300)
            let pairing = (try await mac.status())["pairing"] as? [String: Any]
            XCTAssertEqual(phoneSas, pairing?["sas"] as? String, "both sides must show the same check number")
            let allow = try await mac.cli(["channel", "ios", "allow"])
            XCTAssertEqual(allow.status, 0, allow.stderr)
            try live.release("sas")

            // Notifications allowed: the phone registers the simulator's token with the relay, the Mac gets the handle.
            // The phone first waits for its own side ("On" in the Macs tab), then the handle must reach the Mac.
            let phoneSide = try await live.arrive("push-registered", timeout: 400)
            let registered = await poll(timeout: 90) { await self.pushState() == "ok" }
            numbers["pushRegistration"] = phoneSide
            XCTAssertTrue(registered, "the Mac holds no push handle for this phone (\(phoneSide))")
            try live.release("push-registered", registered ? "ok" : "missing")

            // A question in the foreground, answered in the UI.
            _ = try await live.arrive("ask-single")
            let asked = Date()
            let single = try mac.ask(["-q", "Which build?", "-o", "Debug", "-o", "Release"])
            try live.release("ask-single", String(Int(asked.timeIntervalSince1970 * 1000)))
            numbers["foregroundAnswer"] = try await live.arrive("ask-single-answered", timeout: 300)
            let singleStatus = await single.wait(timeout: 30)
            XCTAssertEqual(singleStatus, 0, single.stderr)
            XCTAssertTrue(single.stdout.contains("Release") && !single.stdout.contains("Debug"), "answer: \(single.stdout)")
            try live.release("ask-single-answered")

            // Two questions, a free text, a shared context.
            _ = try await live.arrive("ask-multi")
            let multi = try mac.ask([
                "Shared context for both", "--single",
                "-q", "First?", "-o", "Yes", "-o", "No",
                "-q", "Second?", "-o", "Red", "-o", "Blue",
            ])
            try live.release("ask-multi")
            _ = try await live.arrive("ask-multi-answered", timeout: 300)
            let multiStatus = await multi.wait(timeout: 30)
            XCTAssertEqual(multiStatus, 0, multi.stderr)
            for expected in ["Yes", "because", "Blue"] { XCTAssertTrue(multi.stdout.contains(expected), "missing \(expected): \(multi.stdout)") }
            try live.release("ask-multi-answered")

            // The permission hook that Agents call asks on the phone: one prompt is allowed, one refused with a reason.
            _ = try await live.arrive("permission-allow")
            let allowed = try mac.ask(["__permission-hook", "claude"], stdin: Self.permissionPayload)
            try live.release("permission-allow")
            _ = try await live.arrive("permission-allow-done", timeout: 300)
            let allowedStatus = await allowed.wait(timeout: 30)
            XCTAssertEqual(allowedStatus, 0, allowed.stderr)
            XCTAssertTrue(allowed.stdout.contains("\"allow\""), "hook output: \(allowed.stdout)")
            try live.release("permission-allow-done")
            _ = try await live.arrive("permission-deny")
            let refused = try mac.ask(["__permission-hook", "claude"], stdin: Self.permissionPayload)
            try live.release("permission-deny")
            _ = try await live.arrive("permission-deny-done", timeout: 300)
            let refusedStatus = await refused.wait(timeout: 30)
            XCTAssertEqual(refusedStatus, 0, refused.stderr)
            XCTAssertTrue(refused.stdout.contains("deny"), "hook output: \(refused.stdout)")
            XCTAssertTrue(refused.stdout.contains("Run the tests first."), "the reason did not reach the Agent: \(refused.stdout)")
            try live.release("permission-deny-done")

            // An Agent session on the Mac, as the Agent's own hooks report it: the phone lists it, watches it
            // live, follows it and queues a message that the Agent's next step has to carry.
            _ = try await live.arrive("agent-start")
            try await hook("session-start")
            try await hook("turn-start")
            try live.release("agent-start")
            _ = try await live.arrive("agent-message", timeout: 300)
            // The phone shows the message as queued once the Mac took it, but allow for a slow link anyway:
            // every step asks the Mac for what is waiting, and only the first one gets the message.
            var carried = ""
            for _ in 0..<40 {
                carried = try await hook("activity", extra: toolCall("cargo test")).stdout
                if carried.contains(Self.message) { break }
                try await Task.sleep(nanoseconds: 500_000_000)
            }
            XCTAssertTrue(carried.contains(Self.message), "the Agent's step did not carry the message: \(carried)")
            try live.release("agent-message")
            // The turn ends while the phone shows the list of Agents: a followed Agent finishing is announced.
            _ = try await live.arrive("agent-finish", timeout: 300)
            try await hook("turn-end")
            try live.release("agent-finish")
            _ = try await live.arrive("agent-finished", timeout: 300)
            try live.release("agent-finished")

            // The app is away: the question reaches the phone as a push (real relay, real APNs sandbox).
            // First, how long the Mac takes to notice that the phone left (Home pressed → "foreground: false").
            _ = try await live.arrive("leaving", timeout: 300)
            let homePressed = Date()
            let left = await poll(timeout: 30) { (await self.devices().first?["foreground"] as? Bool) == false }
            XCTAssertTrue(left, "the Mac never saw the phone leave the foreground")
            numbers["macNoticesLeavingMs"] = Int(Date().timeIntervalSince(homePressed) * 1000)
            _ = try await live.arrive("away", timeout: 240)
            let pushedAt = Date()
            let pushed = try mac.ask(["-q", "Pushed question?", "-o", "Yes", "-o", "No"])
            try live.release("away", String(Int(pushedAt.timeIntervalSince1970 * 1000)))
            let banner = try await live.arrive("pushed", timeout: 300)
            numbers["pushBanner"] = banner
            XCTAssertTrue(banner.contains("Pushed question"), "the notification extension did not show the decrypted question: \(banner)")
            try live.release("pushed")
            _ = try await live.arrive("pushed-answered", timeout: 300)
            let pushedStatus = await pushed.wait(timeout: 30)
            XCTAssertEqual(pushedStatus, 0, pushed.stderr)
            XCTAssertTrue(pushed.stdout.contains("Yes"), "answer: \(pushed.stdout)")
            try live.release("pushed-answered")

            // A followed Agent that finishes a turn while the phone is away comes as a push, too: a different
            // kind of notification (it opens the Agent, not a request).
            _ = try await live.arrive("leaving-again", timeout: 300)
            let leftAgain = await poll(timeout: 30) { (await self.devices().first?["foreground"] as? Bool) == false }
            XCTAssertTrue(leftAgain, "the Mac never saw the phone leave the foreground the second time")
            _ = try await live.arrive("away-again", timeout: 240)
            let finishedAt = Date()
            try await hook("turn-start")
            try await hook("turn-end")
            try live.release("away-again", String(Int(finishedAt.timeIntervalSince1970 * 1000)))
            let followBanner = try await live.arrive("follow-pushed", timeout: 300)
            numbers["followPushBanner"] = followBanner
            XCTAssertTrue(followBanner.contains("Finished"), "the notification extension did not show what happened to the Agent: \(followBanner)")
            try live.release("follow-pushed")

            // Unpairing on the phone reaches the Mac.
            _ = try await live.arrive("unpaired", timeout: 300)
            let forgotten = await poll(timeout: 30) { await self.devices().isEmpty }
            XCTAssertTrue(forgotten, "the Mac still lists the phone")
            try live.release("unpaired")
        } catch {
            runner.terminate()
            await cleanUp(result: result, derived: derived, udid: udid, code: code)
            throw error
        }

        let status = await runner.wait(timeout: 300)
        let tail = runner.stdout.split(separator: "\n").suffix(40).joined(separator: "\n")
        XCTAssertEqual(status, 0, "xcodebuild:\n\(tail)")
        report(numbers)
        await cleanUp(result: result, derived: derived, udid: udid, code: code)
    }

    // MARK: Mac side helpers

    /// What the UI test types to the Agent (see `AskHumanLiveUITests`).
    private static let message = "Also run the lint."

    /// What Claude Code's permission hook is given.
    private static let permissionPayload = """
    {"hook_event_name":"PermissionRequest","session_id":"live-permission","cwd":"/tmp/live-project","permission_mode":"default","tool_name":"Bash","tool_input":{"command":"rm -rf build","description":"Remove the build folder"}}
    """

    /// Reports an event of an Agent session to the Mac the way the Agent's own hooks do (the hidden
    /// `__agent-hook` command). The session is `claude:live-agent`; no terminal is ever launched.
    @discardableResult
    private func hook(_ event: String, extra: [String: Any] = [:]) async throws -> CmdResult {
        let project = mac.home.appendingPathComponent("project")
        try FileManager.default.createDirectory(at: project, withIntermediateDirectories: true)
        var payload: [String: Any] = ["session_id": "live-agent", "cwd": project.path]
        payload.merge(extra) { _, new in new }
        let data = try JSONSerialization.data(withJSONObject: payload)
        let result = try await mac.cli(["__agent-hook", "claude", event], stdin: String(decoding: data, as: UTF8.self), timeout: 30)
        XCTAssertEqual(result.status, 0, result.stderr)
        return result
    }

    private func toolCall(_ command: String) -> [String: Any] {
        ["hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": ["command": command]]
    }

    private func devices() async -> [[String: Any]] {
        (try? await mac.jsonArray(["channel", "ios", "devices", "--json"])) ?? []
    }

    private func pushState() async -> String? { await devices().first?["push"] as? String }

    private func poll(timeout: TimeInterval, _ check: () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(nanoseconds: 200_000_000)
        }
        return false
    }

    // MARK: simulator and leftovers

    /// A clean device: no keychain items, no notification permission, no app. Uninstalling the app drops
    /// its notification settings; the keychain survives an uninstall, so it is reset. Only on request is
    /// the whole device erased.
    private func resetSimulator(_ udid: String) async throws {
        let env = ProcessInfo.processInfo.environment
        let home = FileManager.default.temporaryDirectory
        let simctl = "/usr/bin/xcrun"
        if env["AH_E2E_SIM_ERASE"] == "1" {
            _ = try? await AHE2ETests.run(simctl, ["simctl", "shutdown", udid], env: env, cwd: home, timeout: 120)
            let erase = try await AHE2ETests.run(simctl, ["simctl", "erase", udid], env: env, cwd: home, timeout: 180)
            guard erase.status == 0 else { throw E2EError("simctl erase failed: \(erase.stderr)") }
        }
        // Booting a booted device fails, which is fine; what matters is that it is up.
        _ = try? await AHE2ETests.run(simctl, ["simctl", "boot", udid], env: env, cwd: home, timeout: 120)
        let boot = try await AHE2ETests.run(simctl, ["simctl", "bootstatus", udid, "-b"], env: env, cwd: home, timeout: 300)
        guard boot.status == 0 else { throw E2EError("the simulator did not boot: \(boot.stderr)") }
        // Not installed (a first run): fine too.
        let bundle = env["AH_E2E_BUNDLE_ID"] ?? "com.naituw.humaninloop.ios"
        _ = try? await AHE2ETests.run(simctl, ["simctl", "uninstall", udid, bundle], env: env, cwd: home, timeout: 120)
        let keychain = try await AHE2ETests.run(simctl, ["simctl", "keychain", udid, "reset"], env: env, cwd: home, timeout: 120)
        guard keychain.status == 0 else { throw E2EError("simctl keychain reset failed: \(keychain.stderr)") }
        await waitUntilQuiet(udid)
    }

    /// Waits until the simulator stops doing work of its own (first boot, indexing, widgets): a UI test
    /// that starts on top of it sees every wait stretch by minutes. Gives up quietly after `timeout`.
    private func waitUntilQuiet(_ udid: String, timeout: TimeInterval = 900) async {
        let env = ProcessInfo.processInfo.environment
        let home = FileManager.default.temporaryDirectory
        let deadline = Date().addingTimeInterval(timeout)
        var quietChecks = 0
        while Date() < deadline, quietChecks < 3 {
            // The simulator's processes are the children of its `launchd_sim`.
            guard let listing = try? await AHE2ETests.run("/bin/ps", ["-Ao", "pid,command"], env: env, cwd: home, timeout: 30),
                  let line = listing.stdout.split(separator: "\n").first(where: { $0.contains("launchd_sim") && $0.contains(udid) }),
                  let pid = line.split(separator: " ").first.map(String.init),
                  let top = try? await AHE2ETests.run(
                      "/usr/bin/top", ["-l", "2", "-s", "3", "-n", "2000", "-stats", "pid,ppid,cpu"], env: env, cwd: home, timeout: 90)
            else { return }
            // The first sample of `top` holds averages since the start of each process; the second is current.
            var sample = 0
            var busy = 0.0
            for row in top.stdout.split(separator: "\n") {
                let columns = row.split(separator: " ")
                if columns.first == "PID" {
                    sample += 1
                } else if sample == 2, columns.count == 3, columns[1] == pid {
                    busy += Double(columns[2]) ?? 0
                }
            }
            // Under half a core, three times in a row.
            quietChecks = busy < 50 ? quietChecks + 1 : 0
        }
    }

    /// Deletes the result bundle and checks that the pairing code is nowhere in what the run left behind.
    private func cleanUp(result: URL, derived: URL, udid: String, code: String) async {
        // Before the pairing code exists nothing secret can be in the result bundle, so a run that failed
        // that early keeps it for diagnosis.
        if code.isEmpty, let keep = ProcessInfo.processInfo.environment["AH_E2E_KEEP_LOGS"], !keep.isEmpty,
           FileManager.default.fileExists(atPath: result.path)
        {
            try? FileManager.default.createDirectory(atPath: keep, withIntermediateDirectories: true)
            let target = URL(fileURLWithPath: keep).appendingPathComponent("ui-run.xcresult")
            try? FileManager.default.removeItem(at: target)
            if (try? FileManager.default.moveItem(at: result, to: target)) != nil { return }
        }
        try? FileManager.default.removeItem(at: result)
        // No code yet (the run failed before pairing started): nothing to look for, and an empty pattern
        // would match every line.
        guard !code.isEmpty else { return }
        let library = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Developer/CoreSimulator/Devices/\(udid)/data/Library")
        for place in [derived.appendingPathComponent("Logs"), library.appendingPathComponent("Keyboard"), library.appendingPathComponent("Logs")]
        where FileManager.default.fileExists(atPath: place.path) {
            // The pattern goes through stdin so that it never shows up in a process listing.
            let found = try? await AHE2ETests.run(
                "/usr/bin/grep", ["-rlaF", "-f", "/dev/stdin", place.path], env: [:], cwd: FileManager.default.temporaryDirectory,
                stdin: code + "\n", timeout: 120
            )
            if found?.status == 0 {
                let files = found?.stdout.split(separator: "\n").map(String.init) ?? []
                XCTFail("the pairing code was left in \(files.count) file(s) under \(place.lastPathComponent); they were deleted")
                for file in files { try? FileManager.default.removeItem(atPath: file) }
            }
        }
    }
}

/// The two halves of the scenario meet through files: `ui/<name>` is written by the UI test when it
/// reaches a point, `host/<name>` by the Mac side when it did its part. The content is a payload.
private final class Meeting: @unchecked Sendable {
    let dir: URL
    /// A named pipe: the pairing code passes through the kernel and never lands in a file.
    private var codePipe: URL { dir.appendingPathComponent("code") }

    init() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("ah-live-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: dir.appendingPathComponent("ui"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dir.appendingPathComponent("host"), withIntermediateDirectories: true)
        guard mkfifo(codePipe.path, 0o600) == 0 else { throw E2EError("mkfifo failed (errno \(errno))") }
    }

    func remove() { try? FileManager.default.removeItem(at: dir) }

    /// Writes the code (and a newline) into the pipe once the UI test has it open for reading.
    func hand(code: String, timeout: TimeInterval = 60) async throws {
        let deadline = Date().addingTimeInterval(timeout)
        var fd: Int32 = -1
        while Date() < deadline {
            // Fails with ENXIO until a reader is there; the non-blocking open keeps us from hanging.
            fd = open(codePipe.path, O_WRONLY | O_NONBLOCK)
            if fd >= 0 { break }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        guard fd >= 0 else { throw E2EError("the UI test never opened the pipe for the pairing code") }
        defer { close(fd) }
        let bytes = Array((code + "\n").utf8)
        var written = 0
        while written < bytes.count {
            let n = bytes[written...].withUnsafeBytes { write(fd, $0.baseAddress, $0.count) }
            if n > 0 {
                written += n
            } else if n < 0, errno == EAGAIN || errno == EINTR {
                try await Task.sleep(nanoseconds: 10_000_000)
            } else {
                throw E2EError("writing the pairing code failed (errno \(errno))")
            }
        }
    }
}

private struct Live {
    let meeting: Meeting
    let runner: Cmd

    /// Waits until the UI test reaches `name` and returns its payload. Fails early when the UI test
    /// is gone.
    func arrive(_ name: String, timeout: TimeInterval = 240) async throws -> String {
        let file = meeting.dir.appendingPathComponent("ui/\(name)")
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let payload = try? String(contentsOf: file, encoding: .utf8) { return payload }
            if !runner.isRunning {
                // One last look: it may have written the file just before it ended.
                if let payload = try? String(contentsOf: file, encoding: .utf8) { return payload }
                // Let the output handlers drain: why the run ended is in its last lines.
                await runner.wait(timeout: 5)
                throw E2EError("the UI test ended before reaching \(name) (reached: \(reached())):\n\(output(lines: 40))")
            }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        throw E2EError("the UI test did not reach \(name) within \(Int(timeout)) s (reached: \(reached())); xcodebuild so far:\n\(output(lines: 30))")
    }

    /// The points the UI test announced so far.
    private func reached() -> String {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: meeting.dir.appendingPathComponent("ui").path)) ?? []
        return names.isEmpty ? "nothing" : names.sorted().joined(separator: ", ")
    }

    private func output(lines: Int) -> String {
        let out = runner.stdout.split(separator: "\n").suffix(lines).joined(separator: "\n")
        let err = runner.stderr.split(separator: "\n").suffix(10).joined(separator: "\n")
        return err.isEmpty ? out : out + "\n-- stderr --\n" + err
    }

    func release(_ name: String, _ payload: String = "") throws {
        try payload.write(to: meeting.dir.appendingPathComponent("host/\(name)"), atomically: true, encoding: .utf8)
    }
}
