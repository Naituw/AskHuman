import SwiftUI

/// A line of text that is waiting for something (docs/specs/ios-session-history.md, S19): gray, with a band of the primary
/// colour sweeping over it from left to right, again and again. No colour of its own: in the light appearance the band is
/// dark, in the dark one it is white. With Reduce Motion the line stands still.
struct ShimmerText: View {
    private let text: Text
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(_ key: LocalizedStringKey) {
        text = Text(key)
    }

    var body: some View {
        text
            .foregroundStyle(.secondary)
            .overlay {
                if !reduceMotion {
                    TimelineView(.animation) { timeline in
                        text
                            .foregroundStyle(.primary)
                            .mask { ShimmerBand(phase: Self.phase(at: timeline.date)) }
                    }
                    // The same words again, only lit: not to be read a second time.
                    .accessibilityHidden(true)
                }
            }
    }

    /// Where the band is on its way across: 0 before the line, 1 past it.
    private static func phase(at date: Date) -> Double {
        let period = 1.5
        return date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: period) / period
    }
}

/// The soft-edged band that lets the primary colour through, as wide as about half the line.
private struct ShimmerBand: View {
    let phase: Double

    var body: some View {
        GeometryReader { geometry in
            let width = geometry.size.width
            let band = width * 0.5
            LinearGradient(colors: [.clear, .black, .clear], startPoint: .leading, endPoint: .trailing)
                .frame(width: band)
                .offset(x: -band + (width + band) * phase)
        }
    }
}
