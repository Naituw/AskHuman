import SwiftUI

/// "2 min ago", refreshed while on screen.
struct RelativeTimeText: View {
    let date: Date

    private static let formatter: RelativeDateTimeFormatter = {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        formatter.dateTimeStyle = .named
        return formatter
    }()

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            Text(label(now: context.date))
        }
    }

    private func label(now: Date) -> String {
        if now.timeIntervalSince(date) < 45 { return String(localized: "now") }
        return Self.formatter.localizedString(for: date, relativeTo: now)
    }
}
