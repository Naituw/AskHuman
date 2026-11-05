import Foundation
import AHProtocol

// Away mode as the screens use it (docs/specs/special-replies.md D31–D36). The state is the Macs': the phone
// asks them and shows what they report (`MacState.away`).

extension AppStore {
    /// The Macs' away states added up.
    public var away: AwayOverview { AwayOverview(macs: state.macs) }

    /// Whether the phone offers away mode at all: some paired Mac has it. Older Macs would drop what is sent.
    public var offersAway: Bool { state.macs.contains { $0.caps.away } }

    /// Turns away mode on for the Macs, with a note the Agents see. Returns the Macs that could not be set (they went
    /// offline meanwhile): the others are on.
    @discardableResult
    public func startAway(macIds: [String], note: String) async -> [String] {
        var failed: [String] = []
        for id in macIds {
            do {
                try await backend.setAway(macId: id, on: true, note: note)
            } catch {
                failed.append(id)
            }
        }
        return failed
    }

    /// Ends the global switch on every Mac that has it on (N9): one tap, no question. Returns the Macs that could not be
    /// reached; the strip stays for those.
    @discardableResult
    public func endAway() async -> [String] {
        var failed: [String] = []
        for mac in away.globalMacs {
            do {
                try await backend.setAway(macId: mac.id, on: false, note: nil)
            } catch {
                failed.append(mac.id)
            }
        }
        return failed
    }

    /// What the session of an Agent does on its own.
    public func sessionMode(of agent: AgentItem) -> AwaySessionMode {
        AwayOverview.mode(of: agent.view, on: state.macs.first { $0.id == agent.macId })
    }

    /// Whether the session's Mac says it is away right now (its own setting, else the Mac's global switch).
    public func isAway(_ agent: AgentItem) -> Bool {
        AwayOverview.isAway(agent: agent.view, on: state.macs.first { $0.id == agent.macId })
    }

    /// Whether the Agent's session can be set from here: its Mac has away mode and answers now.
    public func canSetSessionAway(_ agent: AgentItem) -> Bool {
        guard let mac = state.macs.first(where: { $0.id == agent.macId }) else { return false }
        return mac.caps.away && mac.link == .online
    }

    /// Whether the Mac's global away switch is on (the options of a session's menu depend on it).
    public func isGlobalAway(macId: String) -> Bool {
        state.macs.first { $0.id == macId }?.away?.global != nil
    }

    /// Sets or clears what one session does on its own; it applies at once (D35).
    public func setSessionAway(_ setting: AwaySessionSetting, for agent: AgentItem) async throws {
        try await backend.setSessionAway(macId: agent.macId, sessionId: agent.view.sessionId, setting: setting, note: nil)
    }

    /// Whether the answer bar of this request offers the away directives: the Mac knows them and the request is
    /// a question (a confirmation has no answer to attach them to).
    public func offersDirectives(for item: InboxItem) -> Bool {
        guard item.view?.ask != nil else { return false }
        return state.macs.first { $0.id == item.macId }?.caps.away == true
    }

    /// Whether the question reached the user as an escalation: the Agent asked although the user is away.
    public func isEscalated(_ item: InboxItem) -> Bool {
        item.view?.escalated ?? item.summary.escalated
    }

    /// Whether the Agent waits on a question that came as an escalation (its row says so).
    public func isEscalatedWaiting(_ agent: AgentItem) -> Bool {
        guard agent.view.state == "waiting", let requestId = agent.view.waitingRequestId else { return false }
        return state.inbox.first { $0.macId == agent.macId && $0.requestId == requestId }.map(isEscalated) ?? false
    }
}
