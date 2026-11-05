import SwiftUI
import AHCore
import AHProtocol

// What the away-mode screens say (docs/specs/special-replies.md D31–D36). The words follow the desktop app
// (`src/i18n`), so a phone and a Mac call the same thing by the same name.

enum AwayTexts {
    private static let durationFormatter: DateComponentsFormatter = {
        let formatter = DateComponentsFormatter()
        formatter.allowedUnits = [.day, .hour, .minute]
        formatter.unitsStyle = .abbreviated
        formatter.maximumUnitCount = 2
        formatter.zeroFormattingBehavior = .dropAll
        return formatter
    }()

    /// "1h 23m": how long it has been on, down to the minute.
    static func duration(since date: Date, now: Date) -> String {
        let seconds = max(0, now.timeIntervalSince(date))
        if seconds < 60 { return String(localized: "less than a minute") }
        return durationFormatter.string(from: seconds) ?? ""
    }

    /// The second line of the strip: how long, how many automatic replies so far, and for how many Macs.
    static func stripDetail(_ away: AwayOverview, now: Date) -> String {
        var parts: [String] = []
        if let since = away.since {
            parts.append(String(localized: "For \(duration(since: since, now: now))"))
        }
        if away.replies > 0 {
            parts.append(String(localized: "\(away.replies) automatic replies"))
        }
        if away.globalMacs.count > 1 {
            parts.append(String(localized: "\(away.globalMacs.count) Macs"))
        }
        return parts.joined(separator: " · ")
    }

    /// The state of one session as its row says it: with the global switch on, "normal" is an exception.
    static func sessionStatus(_ mode: AwaySessionMode, globalOn: Bool) -> LocalizedStringKey {
        switch mode {
        case .follow: globalOn ? "Away" : "Normal"
        case .away: "Away"
        case .mute: "Stopped"
        case .normal: "Normal"
        }
    }

    /// The choices of a session's menu. While the global switch is on, "follow" is spelled out as such.
    static func sessionChoices(globalOn: Bool) -> [(setting: AwaySessionSetting, title: LocalizedStringKey)] {
        if globalOn {
            return [(.follow, "Follow away mode"), (.normal, "Always ask normally"), (.mute, "Stop using AskHuman")]
        }
        return [(.follow, "Normal"), (.away, "Away mode"), (.mute, "Stop using AskHuman")]
    }

    /// The choice a session's current setting corresponds to in `sessionChoices`.
    static func current(_ mode: AwaySessionMode, globalOn: Bool) -> AwaySessionSetting {
        switch mode {
        case .follow: .follow
        case .away: globalOn ? .follow : .away
        case .normal: globalOn ? .normal : .follow
        case .mute: .mute
        }
    }

    /// Why a Mac cannot be picked in the start sheet.
    static func unavailable(_ mac: AwayOverview.Entry) -> LocalizedStringKey? {
        if !mac.supported { return "Update AskHuman on this Mac first" }
        if !mac.online { return "Offline, cannot be set now" }
        return nil
    }
}
