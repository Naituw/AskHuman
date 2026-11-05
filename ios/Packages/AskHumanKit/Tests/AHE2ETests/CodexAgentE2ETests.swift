import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The Stop hook the way Codex runs it, fed by hand: there is no Codex in this test, so it runs with the other end-to-end
/// tests. Away mode does not hold back the "ready to end this turn" card (docs/specs/special-replies.md D5), and a
/// session that was muted gets none.
final class CodexStopHookE2ETests: CodexE2ECase {
    func testTheStopHookAsksWhileAwayAndIsSilentWhenMuted() async throws {
        try await pair()
        try await configureTheMac()
        let project = mac.home.appendingPathComponent("project")
        try FileManager.default.createDirectory(at: project, withIntermediateDirectories: true)
        let macId = try unwrap(await phone.state().macs.first?.id)
        try await phone.engine.setAway(macId: macId, on: true, note: nil)
        await eventually("the Mac to be away") { (try? await self.mac.cli(["away", "status"]))?.stdout.contains("on since") == true }

        // What Codex hands to a Stop hook; `session_id` is the key of the mute.
        let session = "e2e-stop-session"
        let payload = """
        {"session_id":"\(session)","transcript_path":"\(mac.home.appendingPathComponent("rollout.jsonl").path)","cwd":"\(project.path)","hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"All done."}
        """
        let stopHook = ["__stop-hook", "codex", "track", "confirm"]

        let first = try mac.ask(stopHook, stdin: payload, cwd: project)
        let card = try await phone.request()
        note("PHONE got \(describe(card))")
        let question = try unwrap(card.view?.ask?.questions.first)
        XCTAssertTrue(question.text.contains("ready to end this turn"), "away mode does not hold back the Stop confirmation")
        XCTAssertFalse(card.view?.escalated ?? true, "a Stop card is not an escalation")
        try await answer(card, option: question.options.firstIndex { $0.text == "End conversation" } ?? 1)
        let firstStatus = await first.wait(timeout: 30)
        note("stop hook (away) exited with \(String(describing: firstStatus)); stdout=\(first.stdout.prefix(200)) stderr=\(first.stderr.prefix(200))")
        XCTAssertEqual(firstStatus, 0)
        await eventually("the card to leave the phone") { await self.phone.state().inbox.isEmpty }

        try await phone.engine.setSessionAway(macId: macId, sessionId: session, setting: .mute, note: nil)
        await eventually("the Mac to know the muted session") {
            ((try? String(contentsOf: self.mac.configDir.appendingPathComponent("state/away.json"), encoding: .utf8)) ?? "").contains(session)
        }
        let second = try mac.ask(stopHook, stdin: payload, cwd: project)
        let secondStatus = await second.wait(timeout: 30)
        note("stop hook (muted) exited with \(String(describing: secondStatus)); stdout=\(second.stdout.prefix(200)) stderr=\(second.stderr.prefix(200))")
        XCTAssertEqual(secondStatus, 0)
        let inbox = await phone.state().inbox
        XCTAssertTrue(inbox.isEmpty, "a muted session gets no Stop card: \(inbox.map { $0.requestId })")
        let lines = decisionLines()
        for line in lines { note("daemon: \(line.prefix(300))") }
        XCTAssertTrue(lines.contains { $0.contains("session_muted") }, "the hook was silenced because of the mute")
    }
}

/// A real Codex (`codex exec`) against a throwaway Mac, answered by the fake phone: the special replies (away mode, "stop
/// using AskHuman", and lifting that again) with a real Agent. They start a real model, so they cost money and take
/// minutes. They run only with `AH_E2E_CODEX=1`, on top of what the other end-to-end tests need:
///
///   AH_E2E_CODEX=1 AH_E2E_BIN=<installed AskHuman> AH_E2E_BROKER_DIR=<ios/Tools/local-broker> \
///   swift test --filter CodexAgentE2ETests
///
/// Optional: `AH_E2E_CODEX_MODEL=<model>` (else Codex's default one), `AH_E2E_CODEX_BIN=<codex>` (else the one on PATH),
/// `AH_E2E_KEEP_LOGS=<folder>` (a failed test also leaves what Codex printed and the timeline there).
///
/// The machine needs Codex logged in and AskHuman's Codex hooks installed (`hooks.json` of the Codex home: they inject the
/// protocol and report Stop and tool activity); the tests skip themselves otherwise. Written and run against
/// codex-cli 0.163.0. What they check, and what they cannot, is in docs/plans/special-replies.md ("真实 Codex 端到端").
///
/// Isolation. Codex gets the real HOME (its login and its hooks are there) but a minimal environment and the throwaway
/// `ASKHUMAN_HOME`, and its MCP server is told that home on the command line. Every hook and tool call therefore lands
/// in the throwaway Mac. Each test ends by checking it: the working directory of the run is named uniquely, and that
/// name has to be found in the throwaway home and nowhere in a real AskHuman home. The Codex config of the user is not
/// read (`--ignore-user-config`; `--dangerously-bypass-hook-trust` lets the hooks run without the trust records that live
/// in it). The threads a run creates are archived afterwards (reversible: Codex moves them to `archived_sessions`).
final class CodexAgentE2ETests: CodexE2ECase {
    private struct CodexRun {
        var label: String
        var cmd: Cmd
    }

    private var codex = ""
    private var workdir: URL!
    private var runs: [CodexRun] = []

    override func setUp() async throws {
        // Everything that can say "skip" comes before the throwaway Mac and the broker are started.
        guard ProcessInfo.processInfo.environment["AH_E2E_CODEX"] == "1" else {
            throw XCTSkip("AH_E2E_CODEX=1 is not set: these tests start a real, paid Codex")
        }
        guard let executable = Self.codexExecutable() else {
            throw XCTSkip("no codex executable (put it on PATH or set AH_E2E_CODEX_BIN)")
        }
        let hooks = (try? String(contentsOf: Self.codexHome().appendingPathComponent("hooks.json"), encoding: .utf8)) ?? ""
        guard hooks.contains("__agent-hook codex session-start") else {
            throw XCTSkip("AskHuman's Codex hooks are not in \(Self.codexHome().path)/hooks.json: the protocol would not reach Codex")
        }
        try await super.setUp()
        codex = executable
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("ah-codex-e2e-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        workdir = folder
        let version = try await AHE2ETests.run(codex, ["--version"], env: codexEnvironment(), cwd: folder, timeout: 30)
        note("\(version.stdout.trimmingCharacters(in: .whitespacesAndNewlines)) at \(codex)")
    }

    override func tearDown() async throws {
        for run in runs where run.cmd.isRunning { run.cmd.terminate() }
        for run in runs { await run.cmd.wait(timeout: 10) }
        keepCodexEvidenceIfFailed()
        await archiveThreads()
        if let workdir { try? FileManager.default.removeItem(at: workdir) }
        try await super.tearDown()
    }

    // MARK: running Codex

    private static func codexHome() -> URL {
        if let custom = ProcessInfo.processInfo.environment["CODEX_HOME"], !custom.isEmpty { return URL(fileURLWithPath: custom) }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".codex")
    }

    private static func codexExecutable() -> String? {
        let env = ProcessInfo.processInfo.environment
        if let custom = env["AH_E2E_CODEX_BIN"], !custom.isEmpty {
            return FileManager.default.isExecutableFile(atPath: custom) ? custom : nil
        }
        let directories = (env["PATH"] ?? "").split(separator: ":").map(String.init) + ["/opt/homebrew/bin", "/usr/local/bin"]
        return directories.map { "\($0)/codex" }.first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// A TOML string for a `-c key=value` override.
    private static func toml(_ text: String) -> String {
        "\"" + text.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
    }

    /// A minimal environment: the real HOME (the login and the hooks are there), the throwaway AskHuman home, and of the
    /// real environment nothing but what Codex itself reads.
    private func codexEnvironment() -> [String: String] {
        let real = ProcessInfo.processInfo.environment
        var env: [String: String] = [:]
        for key in [
            "HOME", "USER", "LOGNAME", "SHELL", "PATH", "LANG", "TMPDIR", "__CF_USER_TEXT_ENCODING",
            "CODEX_HOME", "CODEX_API_KEY", "OPENAI_API_KEY", "OPENAI_BASE_URL",
        ] {
            if let value = real[key] { env[key] = value }
        }
        env["TERM"] = "dumb"
        env["NO_COLOR"] = "1"
        env["ASKHUMAN_HOME"] = mac.configDir.path
        env["ASKHUMAN_NO_KEYCHAIN"] = "1"
        precondition(mac.configDir.path.hasPrefix(FileManager.default.temporaryDirectory.path))
        return env
    }

    /// Starts `codex exec` on a prompt. `exec` never asks for approvals (it fixes the policy to "never"), so there are no
    /// permission cards in these tests.
    private func startCodex(_ prompt: String, label: String) throws -> Cmd {
        var args = [
            "exec", "--ignore-user-config", "--dangerously-bypass-hook-trust", "--skip-git-repo-check", "--json",
            "-s", "read-only", "-C", workdir.path,
            "-c", "model_reasoning_effort=\"low\"",
            "-c", "mcp_servers.askhuman.command=\(Self.toml(environment.askHumanBinary))",
            "-c", "mcp_servers.askhuman.args=[\"mcp\"]",
            "-c", "mcp_servers.askhuman.env={ASKHUMAN_HOME=\(Self.toml(mac.configDir.path)), ASKHUMAN_NO_KEYCHAIN=\"1\"}",
            "-c", "mcp_servers.askhuman.startup_timeout_sec=30",
            "-c", "mcp_servers.askhuman.tool_timeout_sec=900",
            "-c", "mcp_servers.askhuman.default_tools_approval_mode=\"approve\"",
        ]
        if let model = ProcessInfo.processInfo.environment["AH_E2E_CODEX_MODEL"], !model.isEmpty { args += ["-m", model] }
        args.append(prompt)
        let cmd = Cmd(codex, args, env: codexEnvironment(), cwd: workdir)
        try cmd.start()
        runs.append(CodexRun(label: label, cmd: cmd))
        restartClock()
        note("codex started (\(label))")
        return cmd
    }

    /// What Codex did, from its JSON events: tool calls with their replies, commands and its messages.
    private static func steps(in jsonl: String) -> [String] {
        var out: [String] = []
        for line in jsonl.split(separator: "\n") {
            guard let event = try? JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any] else {
                out.append("raw: \(line.prefix(200))")
                continue
            }
            let type = event["type"] as? String ?? "?"
            guard type == "item.completed", let item = event["item"] as? [String: Any] else {
                if type != "item.started" && type != "turn.started" {
                    out.append("event \(type) \(String(describing: event["message"] ?? event["error"] ?? "").prefix(200))")
                }
                continue
            }
            let kind = item["type"] as? String ?? "?"
            switch kind {
            case "mcp_tool_call":
                let args = item["arguments"].flatMap { try? JSONSerialization.data(withJSONObject: $0) }.map { String(decoding: $0, as: UTF8.self) } ?? "-"
                let result = item["result"].flatMap { try? JSONSerialization.data(withJSONObject: $0, options: [.fragmentsAllowed]) }.map { String(decoding: $0, as: UTF8.self) }
                    ?? String(describing: item["error"] ?? "-")
                out.append("MCP \(item["server"] ?? "?").\(item["tool"] ?? "?") status=\(item["status"] ?? "?") args=\(args.prefix(300))\n      result=\(result.prefix(900))")
            case "command_execution":
                out.append("CMD \(item["command"] ?? "?") exit=\(item["exit_code"] ?? "nil") status=\(item["status"] ?? "?") out=\(String(describing: item["aggregated_output"] ?? "").prefix(200))")
            case "agent_message":
                out.append("AGENT \(String(describing: item["text"] ?? "").prefix(600))")
            case "reasoning":
                break
            default:
                out.append("item \(kind) \(String(describing: item).prefix(300))")
            }
        }
        return out
    }

    private static func threadIds(in jsonl: String) -> [String] {
        jsonl.split(separator: "\n").compactMap { line in
            guard let event = try? JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any],
                  event["type"] as? String == "thread.started"
            else { return nil }
            return event["thread_id"] as? String
        }
    }

    /// Puts what the finished run did on the timeline and returns it, one entry per step.
    private func finish(_ run: Cmd) -> [String] {
        let steps = Self.steps(in: run.stdout)
        note("---- what the Agent did ----")
        for step in steps { print("[codex-e2e]   \(step)") }
        if !run.stderr.isEmpty { note("stderr tail: \(run.stderr.suffix(600))") }
        return steps
    }

    // MARK: isolation and cleanup

    /// Where the run reported to: the name of its working directory is unique, and has to be in the throwaway home (the
    /// hooks got there) and in no real AskHuman home; and no real away state may know a session of the run.
    private func assertTheRunStayedInTheThrowawayHome(_ run: Cmd) async throws {
        let marker = workdir.lastPathComponent
        var realHomes = [FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".askhuman")]
        if let custom = ProcessInfo.processInfo.environment["ASKHUMAN_HOME"], !custom.isEmpty {
            realHomes.append(URL(fileURLWithPath: custom))
        }
        realHomes = realHomes.filter { FileManager.default.fileExists(atPath: $0.path) }

        let own = try await AHE2ETests.run("/usr/bin/grep", ["-rl", marker, mac.configDir.path], env: [:], cwd: mac.home, timeout: 60)
        XCTAssertFalse(own.stdout.isEmpty, "nothing of the run reached the throwaway Mac: the hooks did not report to it")
        if !realHomes.isEmpty {
            let real = try await AHE2ETests.run("/usr/bin/grep", ["-rl", marker] + realHomes.map(\.path), env: [:], cwd: mac.home, timeout: 120)
            XCTAssertTrue(real.stdout.isEmpty, "the run left traces in a real AskHuman home: \(real.stdout)")
        }
        for home in realHomes {
            let away = (try? String(contentsOf: home.appendingPathComponent("state/away.json"), encoding: .utf8)) ?? ""
            for id in Self.threadIds(in: run.stdout) {
                XCTAssertFalse(away.contains(id), "the away state of \(home.path) knows a session of the run")
            }
        }
        note("the run stayed in the throwaway home: \(own.stdout.split(whereSeparator: \.isNewline).count) files carry its name, none in the real one")
    }

    /// `AH_E2E_KEEP_LOGS=<folder>`: next to what `E2ECase` keeps, a failed test leaves what Codex printed and the timeline.
    /// Neither holds a secret.
    private func keepCodexEvidenceIfFailed() {
        guard let folder = ProcessInfo.processInfo.environment["AH_E2E_KEEP_LOGS"], !folder.isEmpty,
              (testRun?.totalFailureCount ?? 0) > 0
        else { return }
        let label = name.filter { $0.isLetter || $0.isNumber || $0 == "_" }
        let target = URL(fileURLWithPath: folder).appendingPathComponent("\(label)-codex-\(Int(Date().timeIntervalSince1970))")
        try? FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
        for run in runs {
            try? run.cmd.stdout.write(to: target.appendingPathComponent("\(run.label).jsonl"), atomically: true, encoding: .utf8)
            try? run.cmd.stderr.write(to: target.appendingPathComponent("\(run.label).stderr"), atomically: true, encoding: .utf8)
        }
        try? timeline.joined(separator: "\n").write(to: target.appendingPathComponent("timeline.txt"), atomically: true, encoding: .utf8)
    }

    /// The threads of these runs would pile up in the Codex history of whoever runs the tests: archive them. Best effort.
    private func archiveThreads() async {
        let ids = runs.flatMap { Self.threadIds(in: $0.cmd.stdout) }
        guard !ids.isEmpty else { return }
        let server = CodexAppServer(executable: codex, environment: codexEnvironment())
        do {
            try server.start()
            try await server.initialize()
            for id in ids { try await server.archive(thread: id) }
            note("archived \(ids.count) Codex thread(s)")
        } catch {
            note("could not archive the Codex threads (\(error)); they stay in the Codex history: \(ids)")
        }
        await server.stop()
    }

    /// What the Mac wrote and what the phone was told right after a mute: the state file and the `away.state` push.
    @discardableResult
    private func snapshotTheMute() async -> [AwaySession] {
        let file = mac.configDir.appendingPathComponent("state/away.json")
        await eventually("the mute to be in the state file", timeout: 10) {
            (try? String(contentsOf: file, encoding: .utf8))?.contains("mute") == true
        }
        let text = ((try? String(contentsOf: file, encoding: .utf8)) ?? "(none)").replacingOccurrences(of: "\n", with: " ")
        note("away.json right after the mute: \(text)")
        var sessions: [AwaySession] = []
        await eventually("the phone to see the muted session", timeout: 10) {
            sessions = await self.phone.state().macs.first?.away?.sessions ?? []
            return sessions.contains { $0.mode == "mute" }
        }
        note("the phone sees away sessions: \(sessions.map { "\($0.sessionId.prefix(12)) \($0.mode) replies=\($0.replies)" })")
        return sessions
    }

    // MARK: probe: hooks and tools reach the throwaway daemon, nothing reaches the real one

    func testARunIsIsolatedFromTheRealAskHuman() async throws {
        try await pair()
        try await configureTheMac()
        let run = try startCodex("Reply with the single word OK and nothing else. Do not use any tool.", label: "probe")
        // Without the end-of-turn marker the Stop hook asks whether the turn may end, and the run waits for the answer.
        try await attend(run, timeout: 240) { item in try await self.letTheAgentFinish(item) }
        let steps = finish(run)
        XCTAssertEqual(run.process.terminationStatus, 0, run.stderr)
        XCTAssertTrue(steps.contains { $0.hasPrefix("AGENT") && $0.contains("OK") }, "the agent answered")
        try await assertTheRunStayedInTheThrowawayHome(run)
    }

    // MARK: away mode: the first question is answered by AskHuman, the Agent escalates, the phone answers

    func testAwayModeWithARealCodex() async throws {
        try await pair()
        try await configureTheMac()
        let macId = try unwrap(await phone.state().macs.first?.id)
        try await phone.engine.setAway(macId: macId, on: true, note: "E2E-NOTE back at five")
        await eventually("the Mac to be away") {
            (try? await self.mac.cli(["away", "status"]))?.stdout.contains("E2E-NOTE") == true
        }
        note("phone turned away mode on")

        let prompt = """
        This is an automated test of the AskHuman tools (MCP server "askhuman"). Talk to the user ONLY through those MCP tools, never in plain text.
        Step 1. Call the `ask` tool with the question "Which fruit should the demo use?" and the options "apple" and "pear".
        Step 2. Read the reply and do exactly what it says. In this test you must NOT pick the fruit yourself under any circumstances: only the user may choose it. So if the reply says the user is away, follow the reply's instructions for reaching the user anyway.
        Step 3. Finish the task the way the AskHuman protocol requires for completing work (a report through the `whats_next` tool), and obey its reply.
        Finally print one line: FRUIT=<the fruit the user chose>.
        """
        let run = try startCodex(prompt, label: "away")
        try await attend(run) { item in
            if let ask = item.view?.ask, !ask.whatsNext, self.endOption(of: ask) == nil {
                // The question of the test.
                let apple = ask.questions.first?.options.firstIndex { $0.text == "apple" } ?? 0
                try await self.answer(item, option: apple)
            } else {
                try await self.letTheAgentFinish(item)
            }
        }
        let steps = finish(run)
        let status = try await mac.cli(["away", "status"])
        note("away status after the run: \(status.stdout.trimmingCharacters(in: .whitespacesAndNewlines))")
        try await assertTheRunStayedInTheThrowawayHome(run)

        XCTAssertEqual(run.process.terminationStatus, 0, run.stderr)
        let asks = steps.filter { $0.hasPrefix("MCP askhuman.ask") }
        XCTAssertGreaterThanOrEqual(asks.count, 2, "the first ask was answered by AskHuman, the Agent asked again")
        XCTAssertTrue(asks.first?.contains("E2E-NOTE") == true, "the away note reaches the Agent in the automatic reply")
        XCTAssertTrue(asks.contains { $0.contains("escalate") && $0.contains("true") }, "the Agent escalated explicitly")
        XCTAssertTrue(seenCards.contains { $0.contains("ASK escalated=true") && $0.contains("Which fruit") }, "the escalated question reached the phone")
        XCTAssertFalse(seenCards.contains { $0.contains("ASK escalated=false") && $0.contains("Which fruit") }, "the first question never reached the phone")
        XCTAssertTrue(steps.contains { $0.hasPrefix("AGENT") && $0.contains("FRUIT=apple") }, "the answer of the phone made it to the Agent")
    }

    // MARK: "stop using AskHuman": the phone answers with the directive, the next ask is told so at once

    func testNotUsingAskHumanWithARealCodex() async throws {
        try await pair()
        try await configureTheMac()
        let prompt = """
        This is an automated test of the AskHuman tools (MCP server "askhuman"). Talk to the user ONLY through those MCP tools, never in plain text.
        Step 1. Call the `ask` tool with the question "Shall I continue with plan A?" and the options "yes" and "no". Read its reply carefully.
        Step 2. Whatever the reply said, call the `ask` tool ONE more time with the question "Second check: is there anything else?" (no options). This second call is part of the test of how the tool reacts; read that reply carefully too.
        Step 3. Print exactly two lines: REPLY1=<one short sentence about what the first reply told you> and REPLY2=<one short sentence about what the second reply told you>. Then stop.
        """
        let run = try startCodex(prompt, label: "mute")
        var muted = false
        try await attend(run) { item in
            if item.view?.ask != nil, !muted {
                muted = true
                try await self.answer(item, option: 0, directive: AnswerDirective(kind: "mute", scope: "session"))
                await self.snapshotTheMute()
            } else {
                // Nothing may reach the phone after the mute; if something does, let the Agent finish instead of waiting.
                XCTFail("a muted session reached the phone again: \(self.describe(item))")
                try await self.letTheAgentFinish(item)
            }
        }
        let steps = finish(run)
        let lines = decisionLines()
        for line in lines { note("daemon: \(line.prefix(300))") }
        try await assertTheRunStayedInTheThrowawayHome(run)

        XCTAssertEqual(run.process.terminationStatus, 0, run.stderr)
        XCTAssertTrue(muted, "the first question reached the phone")
        let asks = steps.filter { $0.hasPrefix("MCP askhuman.ask") }
        XCTAssertEqual(asks.count, 2, "both asks were made")
        XCTAssertEqual(seenCards.count, 1, "no card of any kind came after the mute: \(seenCards)")
        XCTAssertTrue(seenCards.first?.contains("Shall I continue") == true, "the card that reached the phone is the first question")
        XCTAssertTrue(lines.contains { $0.contains("session_muted") }, "the Stop hook ran and was silenced because of the mute")
    }

    // MARK: lifting the mute from the phone reaches the running Agent through the interject queue

    func testLiftingTheMuteReachesTheAgentWithARealCodex() async throws {
        try await pair()
        try await configureTheMac()
        let prompt = """
        This is an automated test of the AskHuman tools (MCP server "askhuman").
        Step 1. Call the `ask` tool with the question "Shall I continue with plan A?" and the options "yes" and "no".
        Step 2. Then run these three shell commands, each in its own tool call and with nothing else in between: `sleep 25`, `sleep 25`, `sleep 25`.
        Step 3. Look at everything you were told during step 2 (tool results and notes added to them). If you were told that AskHuman may be used again, call the `ask` tool with the question "Welcome back: which color?" and the options "red" and "blue" and wait for the answer. Otherwise do NOT call AskHuman again.
        Step 4. Print one line: COLOR=<the answer> or COLOR=none.
        """
        let run = try startCodex(prompt, label: "unmute")
        var muted = false
        var lifted = false
        try await attend(run, timeout: 420) { item in
            guard let ask = item.view?.ask, !ask.whatsNext, self.endOption(of: ask) == nil else {
                try await self.letTheAgentFinish(item)
                return
            }
            if !muted {
                muted = true
                try await self.answer(item, option: 0, directive: AnswerDirective(kind: "mute", scope: "session"))
                let sessions = await self.snapshotTheMute()
                if let muteSession = sessions.first(where: { $0.mode == "mute" }) {
                    // The Agent is in the middle of its first sleep by now; the lift must reach it there.
                    try await Task.sleep(nanoseconds: 6_000_000_000)
                    try await self.phone.engine.setSessionAway(macId: item.macId, sessionId: muteSession.sessionId, setting: .follow, note: nil)
                    lifted = true
                    self.note("PHONE lifted the mute (setting follow)")
                }
            } else {
                let red = ask.questions.first?.options.firstIndex { $0.text == "red" } ?? 0
                try await self.answer(item, option: red)
            }
        }
        let steps = finish(run)
        for line in decisionLines() { note("daemon: \(line.prefix(300))") }
        try await assertTheRunStayedInTheThrowawayHome(run)

        XCTAssertEqual(run.process.terminationStatus, 0, run.stderr)
        XCTAssertTrue(lifted, "the mute was lifted from the phone")
        XCTAssertEqual(seenCards.filter { $0.contains("Shall I continue") }.count, 1, "the first question reached the phone: \(seenCards)")
        XCTAssertEqual(seenCards.filter { $0.contains("Welcome back") }.count, 1, "the second question reached the phone after the mute was lifted: \(seenCards)")
        XCTAssertTrue(steps.contains { $0.hasPrefix("AGENT") && $0.contains("COLOR=red") }, "the agent used AskHuman again")
    }
}
