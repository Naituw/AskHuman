import SwiftUI

// The bar at the foot of a screen that has something to act on ("Send", "Submit").
//
// From iOS 26 it is Liquid Glass's kind of bar: nothing but its controls, floating over the content
// with no backing of its own. The scroll view fades out underneath with the system's soft edge effect
// (`safeAreaBar`), which is what keeps the controls readable, and the controls are glass. Before iOS 26
// it is the classic bar: a material under the controls and a hairline on top.

enum BottomBar {
    /// Whether the bar floats (iOS 26 and later) rather than being a band with a material of its own.
    static var floats: Bool {
        if #available(iOS 26, *) { return true }
        return false
    }
}

extension View {
    /// Pins `bar` to the bottom edge: above the keyboard, with the scrolling content ending above it.
    /// Apply it to the scroll view itself, so that the edge effect has something to fade.
    func actionBar<Bar: View>(@ViewBuilder _ bar: () -> Bar) -> some View {
        modifier(ActionBarPin(bar: bar()))
    }

    /// The classic bar's backing: a material with a hairline on top. Nothing from iOS 26, where the
    /// controls float and a backing would be exactly what Liquid Glass leaves out.
    func classicBarBacking() -> some View {
        modifier(ClassicBarBacking())
    }

    /// Text on the bar that is not a control: a glass chip from iOS 26, where it would otherwise sit
    /// straight on the content that scrolls by underneath; as it is before.
    func barStatusChip() -> some View {
        modifier(BarStatusChip())
    }

    /// The bar's main button: tinted glass from iOS 26, a filled capsule before.
    func barPrimaryButton() -> some View {
        modifier(BarButton(prominent: true))
    }

    /// A button beside the main one: plain glass from iOS 26, an outlined capsule before.
    func barSecondaryButton() -> some View {
        modifier(BarButton(prominent: false))
    }
}

/// Keeps the glass of the controls on a bar in one piece: they are drawn together, and glass shapes
/// that get closer than `spacing` to each other would flow into one. Keep the gaps wider than that.
struct GlassGroup<Content: View>: View {
    var spacing: CGFloat = 8
    @ViewBuilder let content: Content

    var body: some View {
        if #available(iOS 26, *) {
            GlassEffectContainer(spacing: spacing) { content }
        } else {
            content
        }
    }
}

private struct ActionBarPin<Bar: View>: ViewModifier {
    let bar: Bar

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            // `.soft` is spelled out: the automatic style is soft on some releases and a harder band
            // with a crisp edge on others.
            content
                .scrollEdgeEffectStyle(.soft, for: .bottom)
                .safeAreaBar(edge: .bottom, spacing: 0) { bar }
        } else {
            content.safeAreaInset(edge: .bottom, spacing: 0) { bar }
        }
    }
}

private struct ClassicBarBacking: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content
        } else {
            content
                .background(.bar)
                .overlay(alignment: .top) { Divider() }
        }
    }
}

private struct BarStatusChip: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content
                .padding(.horizontal, 14)
                .padding(.vertical, 9)
                .glassEffect(.regular, in: .rect(cornerRadius: 22))
        } else {
            content
        }
    }
}

private struct BarButton: ViewModifier {
    let prominent: Bool

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            if prominent {
                content.buttonStyle(.glassProminent).controlSize(.large)
            } else {
                content.buttonStyle(.glass).controlSize(.large)
            }
        } else {
            if prominent {
                content.buttonStyle(.borderedProminent).controlSize(.large)
            } else {
                content.buttonStyle(.bordered).controlSize(.large)
            }
        }
    }
}
