import SwiftUI
import AHCore

struct RequestConnectionNotice {
    let text: LocalizedStringKey
    var warning = false
    var progressing = false

    var color: Color { warning ? .orange : .secondary }
}

extension RequestConnection {
    /// What a request page says about the link to its Mac, or `nil` when the link is ready. What a page promises for the
    /// time the Mac is back depends on what it is waiting to do: `offlineText` is its line for an offline Mac, and
    /// `updatingText` its line for an updating one (the forms' own, about their input being kept, when it is not given).
    func notice(offlineText: LocalizedStringKey, updatingText: LocalizedStringKey? = nil) -> RequestConnectionNotice? {
        switch self {
        case .ready: nil
        case .updating: RequestConnectionNotice(text: updatingText ?? "Your Mac is updating. Your input is kept and sending resumes automatically.", progressing: true)
        case .connecting: RequestConnectionNotice(text: "Connecting…", progressing: true)
        case .reconnecting: RequestConnectionNotice(text: "Connection interrupted. Retrying…", progressing: true)
        case .refused: RequestConnectionNotice(text: "Connection refused. Check the connection settings on your Mac.", warning: true)
        case .macOffline: RequestConnectionNotice(text: offlineText, warning: true)
        case .unavailable: RequestConnectionNotice(text: "Your Mac is not reachable", warning: true)
        }
    }
}
