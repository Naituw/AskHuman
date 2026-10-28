import SwiftUI
import AHCore
import AHProtocol

// What the Todos tab says. The words follow the desktop's todo window (`src/i18n`), so a phone and a Mac
// call the same thing by the same name.

extension TodoNotice {
    /// One sentence for the capsule that shows for a moment.
    var message: String {
        switch self {
        case .changedElsewhere: String(localized: "That todo was already handled elsewhere. The list is up to date now.")
        case .refused(let reason): Self.refusal(reason)
        case .notSaved: String(localized: "The Mac could not save that.")
        case .offline: String(localized: "The Mac is offline. Try again when it is back.")
        case .unconfirmed: String(localized: "The Mac did not confirm that. The list shows what it has now.")
        }
    }

    private static func refusal(_ reason: String) -> String {
        switch reason {
        case "empty": String(localized: "There is nothing to add.")
        case "tooLong": String(localized: "That todo is too long.")
        case "cloudOnly": String(localized: "Auto-run needs a folder on a Mac, and this project has none.")
        case "unknownProject", "notFound": String(localized: "This project is no longer there.")
        default: String(localized: "The Mac refused it (\(reason)).")
        }
    }
}

extension TodoSyncLine {
    /// The status line of an iCloud source.
    @ViewBuilder
    var text: some View {
        switch self {
        case .notYet: Text("Waiting for the first sync")
        case .synced(let date):
            HStack(spacing: 4) {
                Text("Synced ·")
                RelativeTimeText(date: date)
            }
        case .syncing: Text("Syncing…")
        case .waiting(let count): Text("Offline · \(count) waiting to upload")
        case .notSignedIn: Text("Not signed in to iCloud")
        case .accountChanged: Text("Apple ID changed")
        case .zoneMissing: Text("iCloud data was cleared")
        case .quota: Text("iCloud storage is full")
        case .helperNotRunning: Text("Sync helper is not running")
        case .helperIncompatible: Text("Sync helper cannot use iCloud")
        case .helperUnsupported: Text("Sync helper not installed")
        }
    }

    var symbol: String {
        switch self {
        case .synced: "checkmark.icloud"
        case .syncing, .notYet: "arrow.triangle.2.circlepath.icloud"
        case .waiting: "icloud.slash"
        default: "exclamationmark.icloud"
        }
    }
}

extension TodoSourceChoice {
    var titleText: Text {
        switch source {
        case .icloud: Text("iCloud")
        case .mac: Text(verbatim: title ?? "")
        }
    }

    var subtitleText: Text? {
        switch (source, availability) {
        case (.icloud, .noSync): Text("No Mac has iCloud sync on")
        case (.icloud, _): Text("All your synced todos")
        case (.mac, .offline): Text("Offline")
        case (.mac, .needsUpdate): Text("Needs an update")
        case (.mac, .available): syncs ? Text("Online · iCloud sync on") : Text("Online")
        case (.mac, .noSync): nil
        }
    }

    var symbol: String {
        switch source {
        case .icloud: "icloud"
        case .mac: "desktopcomputer"
        }
    }

    /// Whether choosing it can show anything.
    var isChoosable: Bool {
        switch availability {
        case .available, .offline: true
        case .needsUpdate, .noSync: false
        }
    }
}

/// What an empty Todos tab tells, for a source that cannot be shown.
struct TodoFailureTexts {
    var title: LocalizedStringKey
    var detail: Text
    var symbol: String
    var canRetry: Bool

    init(_ failure: TodoFailure, name: String?) {
        let who = name ?? String(localized: "Your Mac")
        switch failure {
        case .noMacs:
            title = "No Mac paired"
            detail = Text("Pair a Mac to see its todos here.")
            symbol = "desktopcomputer"
            canRetry = false
        case .offline:
            title = "Mac not reachable"
            detail = Text("\(who) is not reachable. Its todos show up here when it is back.")
            symbol = "desktopcomputer.trianglebadge.exclamationmark"
            canRetry = false
        case .timedOut:
            title = "The Mac did not answer"
            detail = Text("Pull down to try again.")
            symbol = "hourglass"
            canRetry = true
        case .needsUpdate:
            title = "This Mac needs an update"
            detail = Text("Update AskHuman on \(who) to manage its todos from your phone.")
            symbol = "arrow.down.app"
            canRetry = false
        case .noSyncMac(let anyOnline):
            if anyOnline {
                title = "iCloud sync is off"
                detail = Text("None of your Macs that are online syncs its todos through iCloud. Turn it on in AskHuman's settings on a Mac, or choose a Mac above.")
            } else {
                title = "Your Macs are not reachable"
                detail = Text("iCloud todos are served by a Mac that has sync on. Bring one online, or choose a Mac above.")
            }
            symbol = "icloud.slash"
            canRetry = false
        case .other(let reason):
            title = "Could not load todos"
            detail = Text(verbatim: reason)
            symbol = "exclamationmark.triangle"
            canRetry = true
        }
    }
}
