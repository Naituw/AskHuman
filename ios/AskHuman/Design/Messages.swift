import SwiftUI
import AHCore
import AHProtocol
import AHStore

extension Error {
    /// A sentence for the user (the engine's errors are English constants for logs and tests).
    var userMessage: String {
        guard let error = self as? BackendError else { return localizedDescription }
        switch error {
        case .macOffline: return String(localized: "The Mac is offline. Your draft is kept; try again when it is back.")
        case .unknownRequest: return String(localized: "This request is no longer available.")
        case .unknownMac: return String(localized: "This Mac is no longer paired.")
        case .tooLarge: return String(localized: "That file is too large (limit 10 MB).")
        case .invalid(let reason): return reason
        case .timedOut: return String(localized: "The Mac did not answer in time.")
        case .cancelled: return String(localized: "Cancelled.")
        case .unavailable(let reason): return reason
        case .rejected(let reason): return Self.rejectionMessage(reason)
        }
    }

    /// The short codes a Mac gives when it refuses a message to an Agent.
    private static func rejectionMessage(_ reason: String) -> String {
        switch reason {
        case "notWorking": String(localized: "The Agent is not working right now. Messages can only be sent while it works.")
        case "unsupported": String(localized: "This Agent cannot receive messages from the phone.")
        case "ended": String(localized: "This session has ended.")
        case "notFound": String(localized: "The Mac no longer has this session.")
        case "empty": String(localized: "There is nothing to send.")
        case "tooLong": String(localized: "That message is too long.")
        case "attachment": String(localized: "An attachment could not be used. Remove it and try again.")
        case "conflict", "interrupted": String(localized: "The Mac could not tell whether this was sent. Write it again.")
        default: String(localized: "The Mac refused it (\(reason)).")
        }
    }
}

extension MacLink {
    var title: LocalizedStringKey {
        switch self {
        case .online: "Connected"
        case .connecting: "Connecting…"
        case .macOffline: "Mac offline"
        case .brokerDown: "Broker unreachable"
        }
    }

    var color: Color {
        switch self {
        case .online: .green
        case .connecting: .orange
        case .macOffline: .gray
        case .brokerDown: .red
        }
    }
}

extension HistoryOutcome {
    func title(by: String?) -> String {
        switch self {
        case .answered: String(localized: "Answered")
        case .cancelled: String(localized: "Cancelled")
        case .expired: String(localized: "Expired")
        case .answeredElsewhere:
            by.map { String(localized: "Answered on \($0)") } ?? String(localized: "Answered elsewhere")
        case .dismissed: String(localized: "Dismissed")
        case .ended: String(localized: "Ended")
        }
    }

    var color: Color {
        switch self {
        case .answered: .green
        case .cancelled, .dismissed: .orange
        case .expired, .ended: .gray
        case .answeredElsewhere: .blue
        }
    }

    var symbol: String {
        switch self {
        case .answered: "checkmark.circle.fill"
        case .cancelled, .dismissed: "xmark.circle.fill"
        case .expired: "clock.badge.xmark.fill"
        case .answeredElsewhere: "arrow.triangle.branch"
        case .ended: "flag.checkered"
        }
    }
}

extension ByteCountFormatStyle {
    static var file: ByteCountFormatStyle { .init(style: .file) }
}
