import SwiftUI
import AHCore

/// Which tab asks for notices: the Inbox also tells about the phone's data use and clock.
enum NoticeScope {
    case inbox, agents
}

/// The problems the user can do something about, as one neutral card at the top of a list.
///
/// A connection that is merely coming up is not one of them: the title says "Updating…" for that.
/// The broker only shows up here once it has stayed unreachable for a while (`brokerTrouble`),
/// and what it says stays calm, because the app keeps trying on its own.
struct ConnectionNotices: View {
    @Environment(AppStore.self) private var store
    let scope: NoticeScope
    /// The broker could not be reached for a while; the list tracks it (see `raised`).
    let brokerTrouble: Bool

    var body: some View {
        let notices = notices()
        if !notices.isEmpty {
            Section {
                ForEach(notices, id: \.id) { notice in
                    NoticeRow(id: notice.id, text: notice.text, symbol: notice.symbol, tint: notice.tint)
                }
            }
        }
    }

    private struct Notice {
        var id: String
        var text: LocalizedStringKey
        var symbol: String
        var tint: Color
    }

    private func notices() -> [Notice] {
        var result: [Notice] = []
        guard store.hasPairedMacs else { return result }
        switch store.syncStatus {
        case .refused:
            result.append(Notice(
                id: "refused",
                text: "The broker refused the connection. Its settings may have changed, or this month's free data allowance may be used up. Check on your Mac, then pair again.",
                symbol: "exclamationmark.octagon.fill", tint: .red
            ))
        case .reconnecting where brokerTrouble:
            result.append(Notice(
                id: "broker", text: "Can't reach the broker, so this may be out of date. Pull down to try again.",
                symbol: "wifi.exclamationmark", tint: .secondary
            ))
        default:
            break
        }
        if scope == .inbox {
            if let traffic = store.state.traffic {
                switch traffic.level {
                case .ok: break
                case .warning:
                    result.append(Notice(
                        id: "traffic", text: "This month's data use is close to the broker's free allowance. The service pauses when it runs out.",
                        symbol: "gauge.with.dots.needle.67percent", tint: .orange
                    ))
                case .exhausted:
                    result.append(Notice(
                        id: "traffic", text: "This month's free data allowance may be used up, so the broker may refuse connections.",
                        symbol: "gauge.with.dots.needle.100percent", tint: .red
                    ))
                }
            }
            if store.state.clockSkew {
                result.append(Notice(
                    id: "clock", text: "This phone's clock differs too much from a Mac's, so some messages are ignored. Check Date & Time in Settings.",
                    symbol: "clock.badge.exclamationmark.fill", tint: .orange
                ))
            }
        }
        let offline = store.macs.filter { $0.link == .macOffline }
        switch scope {
        case .inbox:
            for mac in offline where store.state.inbox.contains(where: { $0.macId == mac.id }) {
                result.append(Notice(
                    id: "offline-" + mac.id, text: "\(mac.name) is offline. Its requests cannot be answered until it is back.",
                    symbol: "desktopcomputer.trianglebadge.exclamationmark", tint: .secondary
                ))
            }
        case .agents:
            // When no Mac is reachable and nothing is listed, the empty state already says so.
            if offline.count < store.macs.count || !store.state.agents.isEmpty {
                for mac in offline {
                    result.append(Notice(
                        id: "offline-" + mac.id, text: "\(mac.name) is not reachable. Its sessions are hidden until it is back.",
                        symbol: "desktopcomputer.trianglebadge.exclamationmark", tint: .secondary
                    ))
                }
            }
        }
        return result
    }
}
