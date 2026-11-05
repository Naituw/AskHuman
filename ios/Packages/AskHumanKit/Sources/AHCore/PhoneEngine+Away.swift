import Foundation
import AHProtocol

// Away mode commands (docs/specs/special-replies.md D31–D34, N9). Each Mac keeps its own state, so
// the phone talks to the Macs one by one; the answer is the Mac's `away.state` push, which lands in
// `MacState.away` like any change made on the Mac itself.

extension PhoneEngine {
    public func setAway(macId: String, on: Bool, note: String?) async throws {
        try requireAwayMac(macId)
        let message = AwaySet(clientMsgId: Bytes.newMsgId(), on: on, note: AwayNote.clean(note))
        guard await send(message, to: macId) else { throw BackendError.macOffline }
    }

    public func setSessionAway(macId: String, sessionId: String, setting: AwaySessionSetting, note: String?) async throws {
        try requireAwayMac(macId)
        let message = AwaySessionSet(
            clientMsgId: Bytes.newMsgId(), sessionId: sessionId, setting: setting, note: AwayNote.clean(note)
        )
        guard await send(message, to: macId) else { throw BackendError.macOffline }
    }
}

extension PhoneEngine {
    /// The Mac answers and has away mode: an older Mac would take the message for noise and the user would think it was on.
    private func requireAwayMac(_ macId: String) throws {
        guard let mac = macs[macId], mac.sync == .synced else { throw BackendError.macOffline }
        guard (mac.caps ?? mac.record.caps).away else { throw BackendError.invalid("This Mac has no away mode.") }
    }
}

/// The note a user types for the Agents: trimmed, and blank means none.
public enum AwayNote {
    /// The Mac keeps at most this many characters (the engine's `NOTE_MAX_CHARS`).
    public static let maxCharacters = 500

    public static func clean(_ raw: String?) -> String? {
        guard let trimmed = raw?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty else { return nil }
        return String(trimmed.prefix(maxCharacters))
    }
}
