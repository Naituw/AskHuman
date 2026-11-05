import Foundation
import AHProtocol
import AHCore

/// Sample away state for the demo mode and the UI tests.
extension MockData {
    /// The factory quick replies a Mac offers with its questions (docs/specs/special-replies.md D27).
    public static let quickReplies = [
        "Continue",
        "Use your own judgement, don't wait for me",
        "Skip this one and work on something else",
        "Explain in more detail",
        "None of these fit, try another approach",
    ]

    /// Gives the questions the quick replies, as a Mac that has away mode does: every question but What's next and a
    /// strict choice carries them.
    static func applyQuickReplies(to state: inout EngineState) {
        for index in state.inbox.indices {
            guard var ask = state.inbox[index].view?.ask, !ask.whatsNext, !ask.selectOnly else { continue }
            ask.quickReplies = quickReplies
            state.inbox[index].view?.ask = ask
        }
    }

    /// The question the `awayOn` scenario shows as an escalation.
    public static let escalatedRequestId = "req-migration"

    /// Puts the Home MacBook Pro into away mode: on for 40 minutes with a note and seven automatic replies, the idle Codex
    /// session muted, the flaky-tests session asking normally anyway, and the migration question an escalation.
    static func applyAwayOn(to state: inout EngineState, now: Date) {
        let sessions = [
            AwaySession(sessionId: "cdx-notes", mode: "mute", sinceMs: ms(now.addingTimeInterval(-1_800))),
            AwaySession(sessionId: "cc-flaky", mode: "normal", sinceMs: ms(now.addingTimeInterval(-900))),
        ]
        let away = AwayState(
            rev: 1,
            global: AwayGlobal(
                sinceMs: ms(now.addingTimeInterval(-2_400)), note: "I'm in a meeting: go with your best judgement.", replies: 7
            ),
            sessions: sessions, escalatedWaiting: 1
        )
        if let home = state.macs.firstIndex(where: { $0.id == homeMacId }) { state.macs[home].away = away }
        if let item = state.inbox.firstIndex(where: { $0.requestId == escalatedRequestId }) {
            state.inbox[item].summary.escalated = true
            state.inbox[item].view?.escalated = true
        }
    }
}
