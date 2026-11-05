import Foundation
import AHProtocol
import AHCore

// Away mode in the mock: the Macs keep a state of their own, changes publish it like the real Macs' pushes.

extension MockBackend {
    public func setAway(macId: String, on: Bool, note: String?) async throws {
        try requireAwayMac(macId)
        awayCalls.append(AwayCall(macId: macId, sessionId: nil, setting: on ? "on" : "off", note: AwayNote.clean(note)))
        await pause(0.2)
        changeAway(macId) { away in
            away.global = on ? AwayGlobal(sinceMs: MockData.ms(now()), note: AwayNote.clean(note), replies: 0) : nil
        }
    }

    public func setSessionAway(macId: String, sessionId: String, setting: AwaySessionSetting, note: String?) async throws {
        try requireAwayMac(macId)
        awayCalls.append(AwayCall(macId: macId, sessionId: sessionId, setting: setting.rawValue, note: AwayNote.clean(note)))
        await pause(0.2)
        changeAway(macId) { away in
            away.sessions.removeAll { $0.sessionId == sessionId }
            guard setting != .follow else { return }
            away.sessions.append(AwaySession(
                sessionId: sessionId, mode: setting.rawValue, sinceMs: MockData.ms(now()), note: AwayNote.clean(note)
            ))
        }
    }

    /// What an answer's directive does once the answer is taken: away mode on, or the session muted.
    func applyDirective(_ directive: AnswerDirective, macId: String, request: InboxItem) {
        let session = state.agents.first { $0.macId == macId && $0.view.waitingRequestId == request.requestId }?.view.sessionId
        changeAway(macId) { away in
            switch (directive.kind, directive.scope) {
            case ("away", "global"):
                away.global = AwayGlobal(sinceMs: MockData.ms(now()), note: directive.note, replies: 0)
            case ("away", _):
                guard let session else { return }
                away.sessions.removeAll { $0.sessionId == session }
                away.sessions.append(AwaySession(sessionId: session, mode: "away", sinceMs: MockData.ms(now()), note: directive.note))
            case ("mute", _):
                guard let session else { return }
                away.sessions.removeAll { $0.sessionId == session }
                away.sessions.append(AwaySession(sessionId: session, mode: "mute", sinceMs: MockData.ms(now())))
            default: break
            }
        }
    }

    private func requireAwayMac(_ macId: String) throws {
        guard let mac = state.macs.first(where: { $0.id == macId }), mac.link == .online, !unreachable.contains(macId) else {
            throw BackendError.macOffline
        }
        guard mac.caps.away else { throw BackendError.invalid("This Mac has no away mode.") }
    }

    func changeAway(_ macId: String, _ change: (inout AwayState) -> Void) {
        guard let index = state.macs.firstIndex(where: { $0.id == macId }) else { return }
        var away = state.macs[index].away ?? AwayState(rev: 0)
        change(&away)
        away.rev += 1
        state.macs[index].away = away
        publish()
    }
}
