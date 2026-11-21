import SwiftUI
import AHCore

struct RequestConnectionNotice {
    let text: LocalizedStringKey
    var warning = false
    var progressing = false
    /// What the line offers to do about it, said at its end. Only an offline Mac has one: "Remove for now"
    /// (docs/specs/ios-offline-remove.md). The page puts it up for confirmation (`AppStore.removalPrompt`).
    var action: LocalizedStringKey?

    var color: Color { warning ? .orange : .secondary }
}

extension RequestConnection {
    /// What a request page says about the link to its Mac, or `nil` when the link is ready. What a page promises for the
    /// time the Mac is back depends on what it is waiting to do: `offlineText` is its line for an offline Mac, and
    /// `updatingText` its line for an updating one (the forms' own, about their input being kept, when it is not given).
    /// `removable` adds the way out of an offline Mac to its line: the request can be removed for now.
    func notice(
        offlineText: LocalizedStringKey, updatingText: LocalizedStringKey? = nil, removable: Bool = false
    ) -> RequestConnectionNotice? {
        switch self {
        case .ready: nil
        case .updating: RequestConnectionNotice(text: updatingText ?? "Your Mac is updating. Your input is kept and sending resumes automatically.", progressing: true)
        case .connecting: RequestConnectionNotice(text: "Connecting…", progressing: true)
        case .reconnecting: RequestConnectionNotice(text: "Connection interrupted. Retrying…", progressing: true)
        case .refused: RequestConnectionNotice(text: "Connection refused. Check the connection settings on your Mac.", warning: true)
        case .macOffline: RequestConnectionNotice(text: offlineText, warning: true, action: removable ? "Remove for now" : nil)
        case .unavailable: RequestConnectionNotice(text: "Your Mac is not reachable", warning: true)
        }
    }
}

/// The end of a line about an offline Mac that offers something to do: the action in the accent colour, with the
/// chevron of a line that leads somewhere. The line is the button (a chip on the bar, a row on the loading page).
struct NoticeActionWords: View {
    let action: LocalizedStringKey

    var body: some View {
        HStack(spacing: 2) {
            Text(action).fontWeight(.semibold).fixedSize()
            Image(systemName: "chevron.right").font(.caption2.weight(.bold))
        }
        .foregroundStyle(Color.accentColor)
        .padding(.leading, 3)
    }
}
