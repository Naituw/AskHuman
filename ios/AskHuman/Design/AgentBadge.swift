import SwiftUI
import AHProtocol

/// The mark of an Agent on a rounded tile in its brand color; an unknown kind gets a plain terminal tile.
struct AgentBadge: View {
    let agent: AgentRef
    var size: CGFloat = 36

    var body: some View {
        let style = AgentStyle(kind: agent.kind)
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
                .fill(style.tile)
            // A hairline keeps the dark tiles apart from a dark background.
            RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
                .strokeBorder(Color.white.opacity(0.12), lineWidth: max(0.5, size / 72))
            mark(style)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }

    @ViewBuilder
    private func mark(_ style: AgentStyle) -> some View {
        if let logo = style.logo {
            Image(logo)
                .resizable()
                .renderingMode(style.tintsLogo ? .template : .original)
                .scaledToFit()
                .foregroundStyle(.white)
                .frame(width: size * style.markScale, height: size * style.markScale)
        } else {
            Image(systemName: "terminal.fill")
                .font(.system(size: size * 0.46, weight: .semibold))
                .foregroundStyle(.white)
        }
    }
}

/// How an Agent kind looks: the logo in the asset catalog (`AgentLogos`, see `ios/Tools/agent-logos`)
/// and the tile behind it.
struct AgentStyle {
    /// Asset name of the logo; `nil` for a kind without one.
    let logo: String?
    let tile: AnyShapeStyle
    /// The logo is a one-color shape painted white; otherwise it keeps its own colors.
    let tintsLogo: Bool
    /// Width of the logo relative to the tile.
    let markScale: CGFloat

    private static let night = LinearGradient(
        colors: [Color(red: 0.23, green: 0.23, blue: 0.25), Color(red: 0.09, green: 0.09, blue: 0.10)],
        startPoint: .top, endPoint: .bottom
    )

    init(kind: String) {
        switch kind {
        case "claude":
            self.init(logo: "agent-claude", tile: Color(red: 0.85, green: 0.47, blue: 0.34).gradient, markScale: 0.56)
        case "cursor":
            self.init(logo: "agent-cursor", tile: Self.night, markScale: 0.52)
        case "codex":
            self.init(
                logo: "agent-codex",
                tile: LinearGradient(
                    colors: [Color(red: 0.56, green: 0.66, blue: 1), Color(red: 0.22, green: 0.25, blue: 1)],
                    startPoint: .top, endPoint: .bottom
                ),
                markScale: 0.58
            )
        case "grok":
            self.init(logo: "agent-grok", tile: Color(red: 0.03, green: 0.03, blue: 0.03).gradient, markScale: 0.52)
        case "pi":
            self.init(logo: "agent-pi", tile: Self.night, tintsLogo: false, markScale: 0.48)
        default:
            self.init(logo: nil, tile: Color.gray.gradient, markScale: 0.5)
        }
    }

    private init<S: ShapeStyle>(logo: String?, tile: S, tintsLogo: Bool = true, markScale: CGFloat) {
        self.logo = logo
        self.tile = AnyShapeStyle(tile)
        self.tintsLogo = tintsLogo
        self.markScale = markScale
    }
}
