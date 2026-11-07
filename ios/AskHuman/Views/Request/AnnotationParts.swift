import SwiftUI

/// The small purple number of a note, as the message page draws it beside the passage, in the list of notes and in the
/// history (docs/specs/message-annotations.md D18). Hidden from VoiceOver: the row it is in says what it needs to.
struct AnnotationNumber: View {
    /// `nil` for a note whose passage the page could not find: it has no number on the page, so it gets a dash.
    let number: Int?

    @ScaledMetric(relativeTo: .footnote) private var size: CGFloat = 22

    var body: some View {
        Group {
            if let number {
                Text(verbatim: "\(number)")
            } else {
                Text(verbatim: "–")
            }
        }
        .font(.footnote.weight(.bold))
        .monospacedDigit()
        .foregroundStyle(Color.purple)
        .frame(width: size, height: size)
        .background(Color.purple.opacity(0.14), in: Circle())
        .accessibilityHidden(true)
    }
}

/// How many notes there are, on the corner of the bar's round "Annotations" button: white on purple. It takes no taps
/// (the button under it does) and says nothing to VoiceOver (the button's label has the count).
struct AnnotationCountBadge: View {
    let count: Int

    var body: some View {
        Text(verbatim: "\(count)")
            .font(.caption2.weight(.bold))
            .monospacedDigit()
            .foregroundStyle(.white)
            .padding(.horizontal, 5)
            .frame(minWidth: 18, minHeight: 18)
            .background(Color.purple, in: Capsule())
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }
}
