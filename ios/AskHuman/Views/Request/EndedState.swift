import SwiftUI
import AHCore
import AHStore

// What a request page does once its request has ended somewhere else (docs/specs/ios-answer-flow.md §3.7): a strip
// under the navigation bar that says nothing is asked any more, the body fading, the controls going dead, and a tap
// and a spoken notice if it happened while the page was in view. A small card among the other cards at the top of
// the page was read past: people had read on for a while before they saw it, and it scrolled away with the rest.
// The strip also carries the way on (§3.8): a button, and a countdown that presses it.

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
    /// `id` is the request of the page: the strip's button and countdown act on it.
    func endedStrip(_ finished: FinishedRequest?, id: String) -> some View {
        modifier(EndedStripPin(finished: finished, id: id))
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
///
/// Beside the sentence is the way on (docs/specs/ios-answer-flow.md §3.8): one button, "Next request" while another
/// waits and "Close" when none does, and, for the few seconds the countdown runs, a "Stay" button. The countdown presses
/// the first button when it runs out. The sentence stays its own accessibility element (`finished-notice`): the buttons
/// are separate elements next to it.
struct EndedStrip: View {
    let id: String
    let finished: FinishedRequest
    @Environment(AppStore.self) private var store
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// The least the sentence keeps of the width beside the buttons: its longest words fit whole and the title takes two
    /// lines at the most. With less, the buttons go under it. It grows with the text size, as the words do.
    @ScaledMetric(relativeTo: .footnote) private var sentenceWidth = 120.0

    private static let iconWidth: CGFloat = 34
    private static let iconGap: CGFloat = 12

    private var action: EndedAction { store.endedAction(for: id) }
    private var countdown: EndedCountdown? { store.endedCountdown.flatMap { $0.id == id ? $0 : nil } }

    var body: some View {
        // The buttons sit beside the sentence while it keeps room to read; under it when it would not (a narrow screen, a
        // large text size, both buttons showing).
        EndedStripLayout(minLeading: Self.iconWidth + Self.iconGap + sentenceWidth, spacing: 8) {
            HStack(spacing: Self.iconGap) {
                Image(systemName: finished.outcome.symbol)
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(.white)
                    .frame(width: Self.iconWidth, height: Self.iconWidth)
                    .background(.white.opacity(0.22), in: Circle())
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 1) {
                    Text("No reply needed").font(.subheadline.weight(.semibold))
                    // White on orange or grey is not much of a contrast: the sentence is not made smaller or paler.
                    Text(verbatim: finished.reason).font(.footnote)
                }
                .foregroundStyle(.white)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("finished-notice")
            }
            controls
        }
        .padding(.leading, 10)
        .padding(.trailing, 8)
        .padding(.vertical, 4)
        // The strip stays in view while the page scrolls, so it must not take a quarter of the screen at the largest
        // sizes (that is what the largest accessibility size does to it); VoiceOver speaks the text whatever its size.
        .dynamicTypeSize(...DynamicTypeSize.accessibility1)
        .modifier(EndedStripBackground(tint: finished.outcome.color))
        // Mounting the visible strip, rather than receiving a network event, starts its three seconds.
        .task(id: id) { await monitorPresentation() }
    }

    private var controls: some View {
        HStack(spacing: 2) {
            if countdown != nil {
                Button { store.stayOnEnded(id) } label: {
                    EndedChip(prominent: false) { Text("Stay") }
                }
                .buttonStyle(EndedChipPress())
                .accessibilityLabel(Text("Stay on this page"))
                .accessibilityIdentifier("finished-stay")
                .transition(.opacity)
            }
            Button {
                PageEditing.commit()
                store.performEndedAction(for: id)
            } label: {
                primaryLabel
            }
            .buttonStyle(EndedChipPress())
            .accessibilityLabel(action == .close ? Text("Close") : Text("Next request"))
            .accessibilityHint(countdown == nil ? Text(verbatim: "") : Text("It happens by itself in a few seconds. Choose Stay to keep this page."))
            .accessibilityIdentifier("finished-action")
        }
        .fixedSize()
        .animation(.snappy, value: countdown != nil)
    }

    /// The main button: the words say what it does; while the countdown runs they say when, and the white fills with the
    /// strip's colour as the time goes (not with Reduce Motion: a number that changes once a second is enough).
    @ViewBuilder
    private var primaryLabel: some View {
        if let countdown {
            if reduceMotion {
                TimelineView(.periodic(from: countdown.began, by: 1)) { context in
                    countdownChip(countdown, at: context.date, fills: false)
                }
            } else {
                TimelineView(.animation(minimumInterval: 1.0 / 30)) { context in
                    countdownChip(countdown, at: context.date, fills: true)
                }
            }
        } else {
            EndedChip(prominent: true) { if action == .close { Text("Close") } else { Text("Next request") } }
        }
    }

    private func countdownChip(_ countdown: EndedCountdown, at date: Date, fills: Bool) -> some View {
        let left = max(0, countdown.deadline.timeIntervalSince(date))
        let seconds = max(1, Int(left.rounded(.up)))
        return EndedChip(
            prominent: true, fill: fills ? 1 - min(1, left / max(countdown.seconds, 0.001)) : nil, fillTint: finished.outcome.color
        ) {
            if action == .close { Text("Close in \(seconds)s") } else { Text("Next in \(seconds)s") }
        }
    }

    /// What comes over the page while the countdown runs (a sheet, an alert). What was there when the request ended, the
    /// keyboard of what was being written included, is looked at by the store before the countdown starts.
    private func monitorPresentation() async {
        await Task.yield()
        while !Task.isCancelled, store.monitorsEndedNotice(id) {
            if PageEditing.isVisible {
                if PageEditing.isCovered {
                    store.stayOnEnded(id)
                } else {
                    store.endedNoticeDisplayed(id)
                }
            }
            try? await Task.sleep(for: .milliseconds(150))
        }
    }
}

/// The strip's two parts: the icon with the sentence, and the buttons. The buttons sit at the trailing edge, level with
/// the sentence, while the sentence keeps `minLeading` of the width (it takes as many lines as it needs there); when it
/// would not, they sit under it, still at the trailing edge, and the sentence has the whole width. (A stack could do
/// either, not choose: at the larger text sizes on a narrow screen the sentence was left a few words wide.)
private struct EndedStripLayout: Layout {
    let minLeading: CGFloat
    let spacing: CGFloat

    /// The buttons are as wide as they ask for (they do not wrap).
    private func controlsSize(_ subviews: Subviews) -> CGSize { subviews[1].sizeThatFits(.unspecified) }

    private func sideBySide(in width: CGFloat, controls: CGSize) -> Bool { width - controls.width - spacing >= minLeading }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard subviews.count == 2 else { return .zero }
        let controls = controlsSize(subviews)
        guard let width = proposal.width else {
            let lead = subviews[0].sizeThatFits(.unspecified)
            return CGSize(width: lead.width + spacing + controls.width, height: max(lead.height, controls.height))
        }
        if sideBySide(in: width, controls: controls) {
            let lead = subviews[0].sizeThatFits(ProposedViewSize(width: width - controls.width - spacing, height: nil))
            return CGSize(width: width, height: max(lead.height, controls.height))
        }
        let lead = subviews[0].sizeThatFits(ProposedViewSize(width: width, height: nil))
        return CGSize(width: width, height: lead.height + 4 + controls.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard subviews.count == 2 else { return }
        let controls = controlsSize(subviews)
        if sideBySide(in: bounds.width, controls: controls) {
            let leadWidth = bounds.width - controls.width - spacing
            let lead = subviews[0].sizeThatFits(ProposedViewSize(width: leadWidth, height: nil))
            let height = max(lead.height, controls.height)
            subviews[0].place(
                at: CGPoint(x: bounds.minX, y: bounds.minY + (height - lead.height) / 2), anchor: .topLeading,
                proposal: ProposedViewSize(width: leadWidth, height: lead.height)
            )
            subviews[1].place(
                at: CGPoint(x: bounds.maxX, y: bounds.minY + height / 2), anchor: .trailing, proposal: ProposedViewSize(controls)
            )
        } else {
            let lead = subviews[0].sizeThatFits(ProposedViewSize(width: bounds.width, height: nil))
            subviews[0].place(
                at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width, height: lead.height)
            )
            subviews[1].place(
                at: CGPoint(x: bounds.maxX, y: bounds.minY + lead.height + 4), anchor: .topTrailing, proposal: ProposedViewSize(controls)
            )
        }
    }
}

/// A button of the strip: a pill on the strip's colour. The main one is white with dark words (they read on any colour
/// the strip takes); the other is the paler pill the icon sits in. The pill is smaller than a finger: the space around it
/// is part of the button.
private struct EndedChip<Label: View>: View {
    let prominent: Bool
    /// How much of the pill is filled with `fillTint`, 0...1 (the countdown's progress).
    var fill: Double?
    var fillTint: Color = .clear
    @ViewBuilder let label: Label

    var body: some View {
        label
            // Digits of one width: the seconds go down without the chip (and the button beside it) changing width.
            .font(.subheadline.weight(.semibold).monospacedDigit())
            .lineLimit(1)
            .foregroundStyle(prominent ? Color.black.opacity(0.85) : Color.white)
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background {
                ZStack(alignment: .leading) {
                    Capsule().fill(prominent ? Color.white : Color.white.opacity(0.22))
                    if let fill {
                        GeometryReader { proxy in
                            Rectangle().fill(fillTint.opacity(0.3)).frame(width: proxy.size.width * fill)
                        }
                        .clipShape(Capsule())
                    }
                }
            }
            .padding(.vertical, 7)
            .padding(.horizontal, 2)
            .contentShape(Rectangle())
    }
}

private struct EndedChipPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.opacity(configuration.isPressed ? 0.6 : 1)
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
    let id: String

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            // The soft edge effect under the strip keeps what scrolls by from running into it (as under the bottom bar, G2).
            // Without a strip the page's top edge is left as it was.
            content
                .scrollEdgeEffectStyle(finished == nil ? .automatic : .soft, for: .top)
                .safeAreaBar(edge: .top, spacing: 0) { EndedStripSlot(finished: finished, id: id) }
        } else {
            content.safeAreaInset(edge: .top, spacing: 0) { EndedStripSlot(finished: finished, id: id) }
        }
    }
}

/// Where the strip comes and goes: it slides in from above (it only fades with Reduce Motion), and the page's
/// insets follow it.
private struct EndedStripSlot: View {
    let finished: FinishedRequest?
    let id: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            if let finished {
                EndedStrip(id: id, finished: finished)
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
