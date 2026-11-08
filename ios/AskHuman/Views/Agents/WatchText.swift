import SwiftUI
import AHCore

/// One stable paragraph. Only the text is throttled; the surrounding watch keeps updating.
struct WatchText: View {
    let frame: WatchFrameData
    @Binding var pageReading: Bool
    @State private var presentation = WatchTextPresentation()
    @ScaledMetric(relativeTo: .subheadline) private var previewHeight: CGFloat = 84

    private var candidate: WatchLatestText? {
        if frame.supportsLatestText { return frame.latestText }
        guard let text = frame.text, !text.isEmpty else { return nil }
        return WatchLatestText(id: "legacy", revision: text, kind: "assistant", text: text)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let displayed = presentation.displayed {
                if displayed.kind == "thinking" {
                    Text("Thinking").font(.caption.weight(.medium)).foregroundStyle(.secondary)
                }
                paragraph(displayed)
                    .simultaneousGesture(LongPressGesture(minimumDuration: 0.15).onEnded { _ in presentation.lockReading() })
                    .simultaneousGesture(DragGesture(minimumDistance: 1).onChanged { _ in presentation.lockReading() })
                HStack(spacing: 10) {
                    Button {
                        presentation.toggleExpanded(now: .now)
                        if !presentation.expanded { pageReading = false }
                    } label: {
                        if presentation.expanded { Text("Collapse") } else { Text("Expand") }
                    }
                    .accessibilityIdentifier("watch-text-expand")
                    if displayed.truncated || (!presentation.expanded && displayed.kind == "thinking") {
                        Text("Latest excerpt").foregroundStyle(.tertiary)
                    }
                    Spacer(minLength: 0)
                    if presentation.hasNewContent && (presentation.reading || presentation.expanded) {
                        Button("New content") {
                            presentation.updateNow(now: .now)
                            if !presentation.expanded { pageReading = false }
                        }
                        .accessibilityIdentifier("watch-text-update")
                    }
                }
                .font(.caption)
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
            }
        }
        .onChange(of: candidate, initial: true) { _, value in presentation.receive(value, now: .now) }
        .onChange(of: pageReading, initial: true) { _, value in if value { presentation.lockReading() } }
        .task(id: presentation.deadline) {
            guard let deadline = presentation.deadline else { return }
            let delay = max(0, deadline.timeIntervalSinceNow)
            do { try await Task.sleep(for: .seconds(delay)) } catch { return }
            presentation.tick(now: .now)
        }
        .accessibilityIdentifier("watch-text")
    }

    @ViewBuilder
    private func paragraph(_ displayed: WatchLatestText) -> some View {
        if presentation.expanded {
            if displayed.kind == "thinking" {
                Text(verbatim: displayed.text).font(.subheadline).foregroundStyle(.secondary).textSelection(.enabled)
            } else {
                MarkdownView(text: displayed.text)
            }
        } else {
            Group {
                if displayed.kind == "thinking" {
                    let thought = Text(verbatim: displayed.text)
                        .font(.subheadline).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                    ViewThatFits(in: .vertical) {
                        thought
                        thought.frame(height: previewHeight, alignment: .bottomLeading).clipped()
                    }
                } else {
                    MarkdownView(text: displayed.text, baseFont: .subheadline)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .frame(height: previewHeight, alignment: .topLeading)
            .clipped()
        }
    }
}
