import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// What the Codex end-to-end tests have in common: a throwaway Mac set up the way a real one is, a timeline of what
/// happened, and the fake phone answering whatever reaches it. The tests are in `CodexAgentE2ETests.swift`.
class CodexE2ECase: E2ECase {
    private var started = Date()
    private var handled = Set<String>()
    private(set) var timeline: [String] = []
    /// One line per request that reached the phone, in order.
    private(set) var seenCards: [String] = []

    override func setUp() async throws {
        try await super.setUp()
        restartClock()
    }

    /// Adds a line to the timeline. It is printed as well, so that a run shows what is going on.
    func note(_ text: String) {
        let line = String(format: "[%6.1fs] ", Date().timeIntervalSince(started)) + text
        timeline.append(line)
        print("[codex-e2e] \(line)")
    }

    /// Starts the clock of the timeline over (a run of the Agent begins).
    func restartClock() { started = Date() }

    /// The preferences a real setup has: a delegated run (that is what `codex exec` is) may use AskHuman, and permission
    /// approvals and Stop confirmations go through it. They are written to the throwaway home only.
    func configureTheMac() async throws {
        for args in [
            ["agents", "delegated", "codex", "on"], ["agents", "permission", "codex", "on"],
            ["agents", "stop", "codex", "on"],
        ] {
            let result = try await mac.cli(args)
            note("\(args.joined(separator: " ")) -> exit \(result.status)")
            XCTAssertEqual(result.status, 0, result.stderr)
        }
    }

    /// One line about a request on the phone, for the timeline and for assertions.
    func describe(_ item: InboxItem) -> String {
        guard let view = item.view else { return "(no content)" }
        if let ask = view.ask {
            let questions = ask.questions.map { "\($0.text) \($0.options.map(\.text))" }.joined(separator: " | ")
            return "ASK escalated=\(view.escalated) whatsNext=\(ask.whatsNext) message=\"\(ask.message.text.prefix(100))\" questions=[\(questions)] source=\(view.source) agent=\(view.agent.label)"
        }
        if let confirm = view.confirm {
            let choices = confirm.choices.map { "\($0.wireIndex):\($0.label)(\($0.role))" }
            return "CONFIRM title=\"\(confirm.title)\" choices=\(choices) default=\(String(describing: confirm.defaultIndex)) source=\(view.source) agent=\(view.agent.label)"
        }
        return "kind=\(view.kind)"
    }

    /// Answers the first question of a request, with an option, a text, or both, and optionally a directive.
    func answer(_ item: InboxItem, option: Int? = nil, text: String? = nil, directive: AnswerDirective? = nil) async throws {
        _ = try await phone.engine.submitAnswer(
            macId: item.macId, requestId: item.requestId, action: "send",
            answers: [AnswerItem(q: 0, sel: option.map { [$0] } ?? [], text: text)], todoIds: [], directive: directive
        )
        note("PHONE answered option=\(String(describing: option)) text=\(text ?? "-") directive=\(directive.map { "\($0.kind)/\($0.scope)" } ?? "-")")
    }

    /// Lets things go on: the choice of a confirmation that says "allow" or "end", else its default, else its first.
    func approve(_ item: InboxItem) async throws {
        let confirm = try unwrap(item.view?.confirm)
        let words = ["allow", "approve", "yes", "end", "stop", "允许", "批准", "同意", "确认", "结束"]
        let index = confirm.choices.first { choice in words.contains { choice.label.lowercased().contains($0) } }?.wireIndex
            ?? confirm.defaultIndex ?? confirm.choices.first?.wireIndex
        _ = try await phone.engine.submitConfirm(macId: item.macId, requestId: item.requestId, wireIndex: index, dismiss: false, input: nil)
        note("PHONE confirmed with choice \(String(describing: index))")
    }

    /// The option that ends the turn when `ask` is the Stop confirmation ("The Agent is ready to end this turn"), else nil. An
    /// Agent that ends its turn without the end-of-turn marker is asked this, and a phone that ignores the card leaves it
    /// waiting.
    func endOption(of ask: AskView) -> Int? {
        guard let question = ask.questions.first, question.text.contains("ready to end this turn") else { return nil }
        return question.options.firstIndex { $0.text == "End conversation" } ?? question.options.indices.last
    }

    /// What a phone does that only wants the Agent to finish: it ends the turn when asked to, has nothing more to give a
    /// What's next, answers any other question with its first option, and approves what asks for approval.
    func letTheAgentFinish(_ item: InboxItem) async throws {
        guard let view = item.view else { return }
        if let ask = view.ask {
            if ask.whatsNext {
                try await answer(item, text: "Nothing more. You may stop now.")
            } else {
                try await answer(item, option: endOption(of: ask) ?? 0)
            }
        } else if view.confirm != nil {
            try await approve(item)
        }
    }

    /// Answers what reaches the phone until `run` ends; `handle` sees every request once. A run that does not end in time is
    /// terminated and fails the test.
    func attend(_ run: Cmd, timeout: TimeInterval = 480, _ handle: (InboxItem) async throws -> Void) async throws {
        let deadline = Date().addingTimeInterval(timeout)
        while run.isRunning, Date() < deadline {
            for item in await phone.state().inbox where item.view != nil && !handled.contains(item.requestId) {
                handled.insert(item.requestId)
                let text = describe(item)
                seenCards.append(text)
                note("PHONE got \(text)")
                try await handle(item)
            }
            try await Task.sleep(nanoseconds: 250_000_000)
        }
        if run.isRunning {
            note("TIMEOUT: terminating the Agent")
            run.terminate()
            XCTFail("the Agent did not finish in \(Int(timeout)) s")
        } else {
            note("the Agent exited with status \(run.process.terminationStatus)")
        }
    }

    /// The lines of the throwaway daemon's log that tell about Stop, mute, away and silence decisions.
    func decisionLines() -> [String] {
        let log = (try? String(contentsOf: mac.configDir.appendingPathComponent("daemon.log"), encoding: .utf8)) ?? ""
        return log.split(whereSeparator: \.isNewline).map(String.init).filter {
            let line = $0.lowercased()
            return line.contains("session_muted") || line.contains("stop") && (line.contains("hook") || line.contains("confirm"))
                || line.contains("away") || line.contains("silenc")
        }
    }
}
