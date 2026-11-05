import Foundation
import AHProtocol

// What the phone makes of the away state the Macs report (docs/specs/special-replies.md D31–D34, N9):
// every Mac keeps its own state, the phone only adds the Macs up.

/// What one session does on its own, as the user sees it.
public enum AwaySessionMode: String, Sendable, Equatable {
    /// Follows the global setting.
    case follow
    case away
    case mute
    /// Asks normally even while away mode is on.
    case normal
}

/// The directive the user attaches to an answer from the bar menu (docs/specs/special-replies.md D30): armed in the
/// panel above the bar, and applied by the Mac only when that answer is the one that wins.
public struct AnswerDirectiveChoice: Sendable, Equatable {
    public enum Kind: String, Sendable {
        /// Answer, then turn away mode on.
        case away
        /// Answer, then stop using AskHuman in this session.
        case mute
    }

    public var kind: Kind
    /// For away: only the asking session instead of every Agent. Mute is always about the asking session.
    public var sessionOnly: Bool
    /// For away: what the Agents are told with every automatic reply.
    public var note: String

    public init(kind: Kind, sessionOnly: Bool = false, note: String = "") {
        self.kind = kind
        self.sessionOnly = sessionOnly
        self.note = note
    }

    public static let away = AnswerDirectiveChoice(kind: .away)
    public static let mute = AnswerDirectiveChoice(kind: .mute, sessionOnly: true)

    /// The wire form.
    public var wire: AnswerDirective {
        switch kind {
        case .away:
            return AnswerDirective(kind: "away", scope: sessionOnly ? "session" : "global", note: AwayNote.clean(note))
        case .mute:
            return AnswerDirective(kind: "mute", scope: "session")
        }
    }
}

/// The Macs' away states added up, for the strip above the tab bar and the start sheet.
public struct AwayOverview: Equatable, Sendable {
    public struct Entry: Equatable, Sendable, Identifiable {
        public var id: String
        public var name: String
        /// The Mac answers right now.
        public var online: Bool
        /// The Mac has away mode at all (older Macs do not).
        public var supported: Bool
        public var global: AwayGlobal?
        /// Sessions of this Mac that are away on their own / stopped using AskHuman.
        public var awaySessions: Int
        public var muteSessions: Int
        public var escalatedWaiting: Int
    }

    public var macs: [Entry]

    public init(macs: [MacState]) {
        self.macs = macs.map { mac in
            let sessions = mac.away?.sessions ?? []
            return Entry(
                id: mac.id, name: mac.name, online: mac.link == .online, supported: mac.caps.away,
                global: mac.away?.global,
                awaySessions: sessions.filter { $0.mode == "away" }.count,
                muteSessions: sessions.filter { $0.mode == "mute" }.count,
                escalatedWaiting: mac.away?.escalatedWaiting ?? 0
            )
        }
    }

    /// Macs whose global switch is on.
    public var globalMacs: [Entry] { macs.filter { $0.global != nil } }

    /// The global switch is on somewhere: the strip shows, and its "End" turns those off. A session that is away
    /// on its own shows on its row only.
    public var isGlobal: Bool { !globalMacs.isEmpty }

    /// Sessions away on their own, over all Macs.
    public var sessionAway: Int { macs.reduce(0) { $0 + $1.awaySessions } }

    /// The earliest start among the global switches.
    public var since: Date? {
        globalMacs.compactMap(\.global?.sinceMs).min().map { Date(timeIntervalSince1970: Double($0) / 1000) }
    }

    /// Automatic replies handed out by the global switches.
    public var replies: Int { globalMacs.reduce(0) { $0 + ($1.global?.replies ?? 0) } }

    /// Escalated questions that still wait, over all Macs.
    public var escalatedWaiting: Int { macs.reduce(0) { $0 + $1.escalatedWaiting } }

    /// Macs a new global switch can go to: online and with away mode.
    public var startable: [Entry] { macs.filter { $0.online && $0.supported } }

    /// Whether the start control is worth showing at all: some Mac has away mode.
    public var canStart: Bool { macs.contains { $0.supported } }

    /// The note of the global switch; with several Macs, the first one that has a note.
    public var note: String? { globalMacs.compactMap(\.global?.note).first }

    /// What the session of an Agent does on its own, as its Mac last reported it.
    public static func mode(of agent: AgentView, on mac: MacState?) -> AwaySessionMode {
        guard let session = mac?.away?.sessions.first(where: { $0.sessionId == agent.sessionId }) else { return .follow }
        return AwaySessionMode(rawValue: session.mode) ?? .follow
    }

    /// Whether the Agent's session is away right now: its own setting, else the Mac's global switch.
    public static func isAway(agent: AgentView, on mac: MacState?) -> Bool {
        switch mode(of: agent, on: mac) {
        case .away: return true
        case .mute, .normal: return false
        case .follow: return mac?.away?.global != nil
        }
    }
}
