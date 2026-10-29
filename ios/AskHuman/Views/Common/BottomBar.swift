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

    /// The height of a compose bar's row of controls on the floating bar: the field is at least this tall,
    /// and the round buttons beside it are this wide and tall.
    static let rowHeight: CGFloat = 44

    /// The box for the glyph of one of those round buttons. A glass button adds 7 pt around its label on each
    /// side, so with this box the circle comes out `rowHeight` across.
    static let roundGlyphBox: CGFloat = rowHeight - 14
}

extension View {
    /// Pins `bar` to the bottom edge: above the keyboard, with the scrolling content ending above it.
    /// Apply it to the scroll view itself, so that the edge effect has something to fade.
    func actionBar<Bar: View>(@ViewBuilder _ bar: () -> Bar) -> some View {
        modifier(ActionBarPin(bar: bar()))
    }

    /// The classic bar's backing: a material with a hairline on top (the compose bars never had the hairline,
    /// so they leave it out). Nothing from iOS 26, where the controls float and a backing would be exactly
    /// what Liquid Glass leaves out.
    func classicBarBacking(hairline: Bool = true) -> some View {
        modifier(ClassicBarBacking(hairline: hairline))
    }

    /// Text on the bar that is not a control: a glass chip from iOS 26, where it would otherwise sit
    /// straight on the content that scrolls by underneath; as it is before.
    ///
    /// `compact` is a line of caption text on a chip of its own above the controls. Otherwise the chip is
    /// a block in the row of the controls, as tall as a button; with `fillsHeight` it takes the height
    /// of that row (lay the row out with `fixedSize(horizontal: false, vertical: true)`), so that the
    /// chip and the buttons beside it are one height.
    func barStatusChip(compact: Bool = false, fillsHeight: Bool = false) -> some View {
        modifier(BarStatusChip(compact: compact, fillsHeight: fillsHeight))
    }

    /// A compact chip that is also a button: `barStatusChip(compact: true)` in two halves. The label gets
    /// the padding, so that all of the chip is what is tapped, and the button gets the glass. Do not put
    /// the glass inside the label: a tap on such a button did nothing on iOS 26.2 (the page did not scroll
    /// where the hint leads), while it worked on 27.0.
    func barChipButtonLabel() -> some View {
        modifier(BarChipButtonLabel())
    }

    /// The glass of a chip that is a button, around the button (see `barChipButtonLabel()`).
    func barChipButtonGlass() -> some View {
        modifier(BarChipButtonGlass())
    }

    /// More than a line of text on the bar and not a control (the files waiting to be sent): a glass panel
    /// from iOS 26, where it would otherwise sit straight on the content that scrolls by underneath; as it
    /// is before.
    func barPanel() -> some View {
        modifier(BarPanel())
    }

    /// The box around the text field of a compose bar: glass from iOS 26 (a capsule while the text is on one
    /// line, a rounded rectangle once it wraps), the grouped background before. From iOS 26 a tap anywhere
    /// on the box gives the field the focus (`focus`), not just a tap on its line of text: the box is taller.
    func barFieldBox(focus: FocusState<Bool>.Binding) -> some View {
        modifier(BarFieldBox(focus: focus))
    }

    /// A round button beside the field of a compose bar (attach, auto-run): a glass circle from iOS 26, as it
    /// was before. `active` is off for a button that is not one of those (a menu that is also used in a card).
    func barRoundButton(_ active: Bool = true) -> some View {
        modifier(BarRoundButton(active: active))
    }

    /// The glyph on a round button of the floating bar, at a size of its own (the circle does not follow the
    /// text size, so the glyph does not either), in the box that makes the circle `BottomBar.rowHeight` across.
    func barRoundGlyph(weight: Font.Weight = .semibold) -> some View {
        font(.system(size: 17, weight: weight))
            .frame(width: BottomBar.roundGlyphBox, height: BottomBar.roundGlyphBox)
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
    let hairline: Bool

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content
        } else {
            content
                .background(.bar)
                .overlay(alignment: .top) { if hairline { Divider() } }
        }
    }
}

private struct BarStatusChip: ViewModifier {
    let compact: Bool
    let fillsHeight: Bool

    // The radii make a capsule at the usual heights (a large button, a line of caption text) and stay
    // round without clipping the text when the chip grows with the text size.
    static let compactInsets = EdgeInsets(top: 6, leading: 12, bottom: 6, trailing: 12)
    static let compactRadius: CGFloat = 16

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            if compact {
                content
                    .padding(Self.compactInsets)
                    .glassEffect(.regular, in: .rect(cornerRadius: Self.compactRadius))
            } else {
                content
                    .padding(.horizontal, 14)
                    .padding(.vertical, 7)
                    .frame(maxHeight: fillsHeight ? .infinity : nil)
                    .glassEffect(.regular, in: .rect(cornerRadius: 25))
            }
        } else {
            content
        }
    }
}

private struct BarChipButtonLabel: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.padding(BarStatusChip.compactInsets).contentShape(.rect)
        } else {
            content
        }
    }
}

private struct BarChipButtonGlass: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.glassEffect(.regular, in: .rect(cornerRadius: BarStatusChip.compactRadius))
        } else {
            content
        }
    }
}

private struct BarPanel: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content
                .padding(8)
                .glassEffect(.regular, in: .rect(cornerRadius: 22))
        } else {
            content
        }
    }
}

private struct BarFieldBox: ViewModifier {
    let focus: FocusState<Bool>.Binding
    /// False while the field is disabled (a message is on its way): a tap must not leave a focus behind
    /// that the keyboard would pick up when the field is enabled again.
    @Environment(\.isEnabled) private var isEnabled

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .frame(minHeight: BottomBar.rowHeight)
                .contentShape(.rect)
                .onTapGesture { if isEnabled { focus.wrappedValue = true } }
                .glassEffect(.regular, in: .rect(cornerRadius: BottomBar.rowHeight / 2))
        } else {
            content
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        }
    }
}

private struct BarRoundButton: ViewModifier {
    let active: Bool

    func body(content: Content) -> some View {
        if #available(iOS 26, *), active {
            content.buttonStyle(.glass).buttonBorderShape(.circle).controlSize(.regular)
        } else {
            content
        }
    }
}

/// The send button at the end of a compose bar's row: an up arrow (a spinner while it sends) on a tinted
/// glass circle from iOS 26, a filled arrow before. The caller disables it.
struct BarSendButton: View {
    let label: LocalizedStringKey
    let identifier: String
    var sending = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            if #available(iOS 26, *) {
                Group {
                    if sending { ProgressView() } else { Image(systemName: "arrow.up") }
                }
                .barRoundGlyph(weight: .bold)
            } else {
                Group {
                    if sending { ProgressView() } else { Image(systemName: "arrow.up.circle.fill").font(.system(size: 32)) }
                }
                .frame(width: 34, height: 34)
            }
        }
        .modifier(BarSendStyle())
        .accessibilityLabel(label)
        .accessibilityIdentifier(identifier)
    }
}

private struct BarSendStyle: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.buttonStyle(.glassProminent).buttonBorderShape(.circle).controlSize(.regular)
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
