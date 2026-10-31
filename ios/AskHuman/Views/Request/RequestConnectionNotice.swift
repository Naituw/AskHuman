import SwiftUI
import AHCore

struct RequestConnectionNotice {
    let text: LocalizedStringKey
    var warning = false
    var progressing = false

    var color: Color { warning ? .orange : .secondary }
}

extension RequestConnection {
    func notice(offlineText: LocalizedStringKey) -> RequestConnectionNotice? {
        switch self {
        case .ready: nil
        case .connecting: RequestConnectionNotice(text: "Connecting…", progressing: true)
        case .reconnecting: RequestConnectionNotice(text: "Connection interrupted. Retrying…", progressing: true)
        case .refused: RequestConnectionNotice(text: "Connection refused. Check the connection settings on your Mac.", warning: true)
        case .macOffline: RequestConnectionNotice(text: offlineText, warning: true)
        case .unavailable: RequestConnectionNotice(text: "Your Mac is not reachable", warning: true)
        }
    }
}
