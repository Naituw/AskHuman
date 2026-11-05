import SwiftUI
import AHCore
import AHStore

// What a request page does once its request has ended somewhere else (docs/specs/ios-answer-flow.md §3.7): a strip
// under the navigation bar that says nothing is asked any more, the body fading, the controls going dead, and a tap
// and a spoken notice if it happened while the page was in view. A small card among the other cards at the top of
// the page was read past: people had read on for a while before they saw it, and it scrolled away with the rest.

extension FinishedRequest {
    /// Why nothing is asked any more, in a sentence.
    var reason: String {
        switch outcome {
        case .answeredElsewhere: by.map { String(localized: "It was answered on \($0).") } ?? String(localized: "It was answered elsewhere.")
        case .cancelled: String(localized: "The Agent cancelled it.")
        case .expired: String(localized: "It timed out.")
        case .answered: String(localized: "It was answered.")
        case .dismissed: String(localized: "It was dismissed.")
        case .ended: String(localized: "It finished while you were away.")
        }
    }
}

extension View {
    /// Pins the strip of an ended request under the navigation bar (nothing while the request is open).
    /// Apply it to the scroll view itself, as `actionBar`, so that the edge effect has something to fade.
    func endedStrip(_ finished: FinishedRequest?) -> some View {
        modifier(EndedStripPin(finished: finished))
    }

    /// Fades what asks for an answer once the request has ended, so that the page is not read as if it still wanted one.
    func endedDimming(_ ended: Bool) -> some View {
        modifier(EndedDimming(ended: ended))
    }

    /// A tap and a spoken notice when the request ends while the page is open. A page that opens ended says nothing.
    func endedCue(_ finished: FinishedRequest?) -> some View {
        modifier(EndedCue(finished: finished))
    }
}

/// The strip that says a request is over: what became of it gives it its colour and its sentence.
struct EndedStrip: View {
    let finished: FinishedRequest

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: finished.outcome.symbol)
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 34, height: 34)
                .background(.white.opacity(0.22), in: Circle())
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text("No reply needed").font(.subheadline.weight(.semibold))
                // White on orange or grey is not much of a contrast: the sentence is not made smaller or paler.
                Text(verbatim: finished.reason).font(.footnote)
            }
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.leading, 10)
        .padding(.trailing, 14)
        .padding(.vertical, 8)
        // The strip stays in view while the page scrolls, so it must not take a quarter of the screen at the largest
        // sizes (that is what the largest accessibility size does to it); VoiceOver speaks the text whatever its size.
        .dynamicTypeSize(...DynamicTypeSize.accessibility1)
        .modifier(EndedStripBackground(tint: finished.outcome.color))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("finished-notice")
    }
}

/// The strip's body: tinted Liquid Glass from iOS 26, a material under the colour before.
private struct EndedStripBackground: ViewModifier {
    let tint: Color
    private static let radius: CGFloat = 20

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.glassEffect(.regular.tint(tint), in: .rect(cornerRadius: Self.radius))
        } else {
            content
                .background(tint.opacity(0.94), in: RoundedRectangle(cornerRadius: Self.radius, style: .continuous))
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: Self.radius, style: .continuous))
                .shadow(color: tint.opacity(0.35), radius: 10, y: 3)
        }
    }
}

private struct EndedStripPin: ViewModifier {
    let finished: FinishedRequest?

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            // The soft edge effect under the strip keeps what scrolls by from running into it (as under the bottom bar, G2).
            // Without a strip the page's top edge is left as it was.
            content
                .scrollEdgeEffectStyle(finished == nil ? .automatic : .soft, for: .top)
                .safeAreaBar(edge: .top, spacing: 0) { EndedStripSlot(finished: finished) }
        } else {
            content.safeAreaInset(edge: .top, spacing: 0) { EndedStripSlot(finished: finished) }
        }
    }
}

/// Where the strip comes and goes: it slides in from above (it only fades with Reduce Motion), and the page's
/// insets follow it.
private struct EndedStripSlot: View {
    let finished: FinishedRequest?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            if let finished {
                EndedStrip(finished: finished)
                    .readableWidth()
                    .padding(.horizontal, 16)
                    .padding(.top, 6)
                    .padding(.bottom, 8)
                    .frame(maxWidth: .infinity)
                    .modifier(ClassicStripBacking())
                    .transition(reduceMotion ? .opacity : .move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.snappy, value: finished != nil)
    }
}

/// Before iOS 26 the page scrolls under the strip as under any inset, and the edges of the cards would show around
/// it: a bar behind it, as behind the bottom bar (`classicBarBacking`). From iOS 26 the soft edge effect does that.
private struct ClassicStripBacking: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content
        } else {
            content
                .background(.bar, ignoresSafeAreaEdges: [])
                .overlay(alignment: .bottom) { Divider() }
        }
    }
}

private struct EndedDimming: ViewModifier {
    let ended: Bool

    /// What is left of the body. Controls that are disabled are drawn paler by the system on top of this (a plain
    /// button by half, a menu too), so a group of those is left to the system's own dimming (the confirmation's
    /// choices): twice over they would be too faint to read.
    private static let level = 0.6

    func body(content: Content) -> some View {
        content
            .opacity(ended ? Self.level : 1)
            .animation(.easeInOut(duration: 0.3), value: ended)
    }
}

private struct EndedCue: ViewModifier {
    let finished: FinishedRequest?

    func body(content: Content) -> some View {
        content
            .sensoryFeedback(trigger: finished != nil) { was, now in !was && now ? .warning : nil }
            .onChange(of: finished != nil) { was, now in
                guard !was, now, let finished else { return }
                AccessibilityNotification.Announcement(String(localized: "This request has ended.") + " " + finished.reason).post()
            }
    }
}
