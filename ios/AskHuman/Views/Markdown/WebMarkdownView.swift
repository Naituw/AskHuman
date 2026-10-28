import SwiftUI

/// The Markdown body of a request, drawn by a web page that runs the same engine as the Mac popup
/// (markdown-it, and Mermaid for diagrams), so that the same text looks the same on both. One view is
/// one page and one web content process: a screen uses at most one, for its message.
///
/// The page is an optimisation of looks, never a requirement: when it cannot be used (it does not load,
/// its process keeps dying, or `-AHWebMarkdown NO` was given) the native renderer draws the text and the
/// screen works as before.
@MainActor
struct WebMarkdownView: View {
    let text: String
    /// Called once, when the text has been laid out for the first time (after any diagram was drawn), or
    /// when the native renderer took over. The height of the text is known from then on.
    var onSettled: () -> Void = {}

    @Environment(\.launchOptions) private var launch
    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(LinkPresenter.self) private var links
    @State private var height: CGFloat
    @State private var phase = WebPhase.loading
    @State private var settled = false
    /// The page has been starting for a while (web content takes a moment the first time).
    @State private var impatient = false

    init(text: String, onSettled: @escaping () -> Void = {}) {
        self.text = text
        self.onSettled = onSettled
        // A text seen before starts at the height it had, so that coming back to it does not jump.
        _height = State(initialValue: WebHeights.value(for: text) ?? WebHeights.placeholder)
    }

    var body: some View {
        Group {
            if launch.webMarkdown, phase != .failed {
                WebPage(
                    text: text, typeSize: typeSize, timeout: launch.webTimeout, height: $height, phase: $phase,
                    onLink: { links.tapped($0) }, onSettled: settle
                )
                .frame(height: height)
                .opacity(phase == .loading ? 0 : 1)
                .animation(.easeOut(duration: 0.2), value: phase)
                .animation(.easeInOut(duration: 0.25), value: height)
                .overlay(alignment: .topLeading) {
                    if phase == .loading, impatient {
                        ProgressView().controlSize(.small).transition(.opacity)
                    }
                }
                .task(id: phase) {
                    // A page that is quick to start never shows the spinner; a slow one does not leave an empty card.
                    guard phase == .loading else { impatient = false; return }
                    try? await Task.sleep(for: .milliseconds(600))
                    if !Task.isCancelled { impatient = true }
                }
            } else {
                MarkdownView(text: text)
                    .onAppear(perform: settle)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func settle() {
        guard !settled else { return }
        settled = true
        onSettled()
    }
}

/// How far the page is.
enum WebPhase: Equatable {
    /// Not drawn yet: the page is invisible and keeps the height it had last time (or a line's worth).
    case loading
    /// The text is on the page.
    case shown
    /// The page cannot be used: the native renderer draws the text instead.
    case failed
}

/// The heights the pages reported, by text.
@MainActor
enum WebHeights {
    static let placeholder: CGFloat = 24
    /// The most a page may claim; a script that reports more is not believed.
    static let limit: CGFloat = 20000

    private static var values: [Int: CGFloat] = [:]

    private static func key(_ text: String) -> Int {
        var hasher = Hasher()
        hasher.combine(text)
        return hasher.finalize()
    }

    static func value(for text: String) -> CGFloat? { values[key(text)] }

    static func store(_ height: CGFloat, text: String) {
        if values.count > 100 { values.removeAll() }
        values[key(text)] = height
    }
}
