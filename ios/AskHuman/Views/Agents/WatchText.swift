import SwiftUI
import AHCore

/// One stable paragraph. Only the text is throttled; the surrounding watch keeps updating.
struct WatchText: View {
    let frame: WatchFrameData
    @Binding var pageReading: Bool
    @State private var presentation = WatchTextPresentation()
    /// The height the whole text takes at the width it has, as its window reports it.
    @State private var natural: CGFloat = 0
    @State private var page = PageAnchor()
    @ScaledMetric(relativeTo: .subheadline) private var previewHeight: CGFloat = 84
    /// What of the page above the text is kept in sight when closing it brings the page back to it.
    @ScaledMetric(relativeTo: .subheadline) private var revealMargin: CGFloat = 56

    private var candidate: WatchLatestText? {
        if frame.supportsLatestText { return frame.latestText }
        guard let text = frame.text, !text.isEmpty else { return nil }
        return WatchLatestText(id: "legacy", revision: text, kind: "assistant", text: text)
    }

    /// Whether the text has more than the preview shows, so that there is something to open.
    private var overflows: Bool { natural > previewHeight + 1 }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let displayed = presentation.displayed {
                let thinking = displayed.kind == "thinking"
                if thinking {
                    Text("Thinking").font(.caption.weight(.medium)).foregroundStyle(.secondary)
                }
                // A finger that stays on the text or moves over it (to scroll, or to select) is reading it. It is watched, not
                // taken: a `DragGesture` here would stop the page from scrolling under such a finger (iOS 27).
                TextWindow(expanded: presentation.expanded, tail: thinking, height: previewHeight, natural: $natural) {
                    paragraph(displayed)
                }
                .background { ReadingTouch { presentation.lockReading() } }
                .accessibilityIdentifier("watch-text")
                controls(displayed, thinking: thinking)
            }
        }
        .background { PageAnchorMark(anchor: page) }
        .onChange(of: candidate, initial: true) { _, value in presentation.receive(value, now: .now) }
        .onChange(of: pageReading, initial: true) { _, value in if value { presentation.lockReading() } }
        .task(id: presentation.deadline) {
            guard let deadline = presentation.deadline else { return }
            let delay = max(0, deadline.timeIntervalSinceNow)
            do { try await Task.sleep(for: .seconds(delay)) } catch { return }
            presentation.tick(now: .now)
        }
    }

    /// The text, at one size whether it is opened or not (the window decides how much of it shows).
    @ViewBuilder
    private func paragraph(_ displayed: WatchLatestText) -> some View {
        if displayed.kind == "thinking" {
            Text(verbatim: displayed.text).font(.subheadline).foregroundStyle(.secondary).textSelection(.enabled)
        } else {
            MarkdownView(text: displayed.text, baseFont: .subheadline)
        }
    }

    /// Open / close, what the preview is of, and the way to the newest text. A text that fits in the preview has nothing to
    /// open or to say about being an excerpt.
    @ViewBuilder
    private func controls(_ displayed: WatchLatestText, thinking: Bool) -> some View {
        let openable = overflows || presentation.expanded
        let excerpt = displayed.truncated || (thinking && overflows && !presentation.expanded)
        let newer = presentation.hasNewContent && (presentation.reading || presentation.expanded)
        if openable || excerpt || newer {
            HStack(spacing: 10) {
                if openable {
                    Button(action: toggle) {
                        if presentation.expanded { Text("Collapse") } else { Text("Expand") }
                    }
                    .accessibilityIdentifier("watch-text-expand")
                }
                if excerpt {
                    Text("Latest excerpt").foregroundStyle(.tertiary)
                }
                Spacer(minLength: 0)
                if newer {
                    Button("New content", action: showNewest)
                        .accessibilityIdentifier("watch-text-update")
                }
            }
            .font(.caption)
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
        }
    }

    private func toggle() {
        let closing = presentation.expanded
        // A long text that was read down to its end is about to be short again, and its top is up out of sight: the page
        // goes back to it with the text, instead of being left wherever the shorter page happens to end.
        if closing { page.bringTopIntoSight(margin: revealMargin) }
        withAnimation(.snappy(duration: 0.26)) {
            presentation.toggleExpanded(now: .now)
            if closing { pageReading = false }
        }
    }

    private func showNewest() {
        withAnimation(.snappy(duration: 0.26)) {
            presentation.updateNow(now: .now)
            if !presentation.expanded { pageReading = false }
        }
    }
}

/// The text seen through a window of the card. The text is laid out once, at one size, whether the window is closed or open:
/// opening it only makes the window taller, so nothing reflows and the height can animate. Closed, the window shows `height`
/// points of the text from its start or, for a `tail`, from its end, and fades out the edge where the text goes on.
private struct TextWindow<Content: View>: View {
    let expanded: Bool
    /// The end of a thought is its newest part, so a closed window on one shows that end.
    let tail: Bool
    let height: CGFloat
    @Binding var natural: CGFloat
    private let content: Content
    @ScaledMetric(relativeTo: .subheadline) private var fade: CGFloat = 22

    init(expanded: Bool, tail: Bool, height: CGFloat, natural: Binding<CGFloat>, @ViewBuilder content: () -> Content) {
        self.expanded = expanded
        self.tail = tail
        self.height = height
        _natural = natural
        self.content = content()
    }

    private var overflows: Bool { natural > height + 1 }

    var body: some View {
        content
            .frame(maxWidth: .infinity, alignment: .leading)
            .fixedSize(horizontal: false, vertical: true)
            .background {
                GeometryReader { proxy in Color.clear.preference(key: NaturalHeight.self, value: proxy.size.height) }
            }
            .onPreferenceChange(NaturalHeight.self) { natural = $0 }
            // A short text still takes the preview's height, so that what is under the window stays where it is when the
            // text is replaced by one of another length.
            .frame(height: expanded ? max(natural, height) : height, alignment: tail ? .bottomLeading : .topLeading)
            .clipped()
            .mask { edge }
    }

    /// Opaque, but for the closed window's cut edge, which fades out over `fade` points.
    private var edge: some View {
        let cut = expanded || !overflows ? 0 : fade
        return VStack(spacing: 0) {
            if tail { LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom).frame(height: cut) }
            Rectangle()
            if !tail { LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: cut) }
        }
    }
}

private struct NaturalHeight: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}
