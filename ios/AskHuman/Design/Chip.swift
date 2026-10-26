import SwiftUI

/// A small rounded label: a count, a flag or a status. It keeps to one line and shortens its text
/// when the space runs out, so a long label (a tool name, a command) never widens the row it is in.
struct Chip: View {
    let label: Text
    var systemImage: String?
    var tint: Color = .secondary

    init(text: LocalizedStringKey, systemImage: String? = nil, tint: Color = .secondary) {
        label = Text(text)
        self.systemImage = systemImage
        self.tint = tint
    }

    /// A label that is shown exactly as given (not looked up, not read as Markdown).
    init(verbatim: String, systemImage: String? = nil, tint: Color = .secondary) {
        label = Text(verbatim: verbatim)
        self.systemImage = systemImage
        self.tint = tint
    }

    var body: some View {
        HStack(spacing: 3) {
            if let systemImage {
                Image(systemName: systemImage).imageScale(.small)
            }
            label
                .lineLimit(1)
                .truncationMode(.tail)
        }
        .fixedSize(horizontal: false, vertical: true)
        // Next to a greedy sibling the chip keeps its own width and the sibling gives way.
        .layoutPriority(1)
        .font(.caption2.weight(.medium))
        .foregroundStyle(tint)
        .padding(.horizontal, 6)
        .padding(.vertical, 2.5)
        .background(tint.opacity(0.12), in: Capsule())
    }
}

/// A filled dot with a status color.
struct StatusDot: View {
    let color: Color

    var body: some View {
        Circle().fill(color).frame(width: 8, height: 8)
    }
}
