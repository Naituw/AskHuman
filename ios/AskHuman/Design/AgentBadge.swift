import SwiftUI
import AHProtocol

/// The mark of an Agent on a rounded tile in its brand color; an unknown kind gets a plain terminal tile.
struct AgentBadge: View {
    let agent: AgentRef
    var size: CGFloat = 36
    /// Say the Agent's name to VoiceOver. Beside a text that already names the Agent the badge stays
    /// silent; where a session's title took the name's place, the badge is the only one left to tell.
    var spoken = false

    var body: some View {
        let style = AgentStyle(kind: agent.kind)
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
                .fill(style.tile)
            // A hairline keeps the dark tiles apart from a dark background, and a white one from a light one.
            RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
                .strokeBorder(style.hairline, lineWidth: max(0.5, size / 72))
            mark(style)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(!spoken)
        .accessibilityLabel(spoken ? agent.label : "")
    }

    @ViewBuilder
    private func mark(_ style: AgentStyle) -> some View {
        if let logo = style.logo {
            Image(logo)
                .resizable()
                .renderingMode(style.tintsLogo ? .template : .original)
                .scaledToFit()
                .foregroundStyle(style.markPaint)
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
    /// The logo is a one-color shape painted with `markPaint`; otherwise it keeps its own colors.
    let tintsLogo: Bool
    /// What a one-color logo is painted with: white on a tile in the brand's color, the brand's own
    /// gradient on Codex's white tile.
    let markPaint: AnyShapeStyle
    /// The thin outline round the tile: light on a dark tile, dark on a white one.
    let hairline: Color
    /// Width of the logo relative to the tile.
    let markScale: CGFloat

    private static let night = LinearGradient(
        colors: [Color(red: 0.23, green: 0.23, blue: 0.25), Color(red: 0.09, green: 0.09, blue: 0.10)],
        startPoint: .top, endPoint: .bottom
    )

    /// The cloud of the Codex app icon: violet at the top to blue at the bottom (#B1A7FF, #7A9DFF, #3941FF).
    private static let codexCloud = LinearGradient(
        stops: [
            .init(color: Color(red: 0.694, green: 0.655, blue: 1), location: 0),
            .init(color: Color(red: 0.478, green: 0.616, blue: 1), location: 0.5),
            .init(color: Color(red: 0.224, green: 0.255, blue: 1), location: 1),
        ],
        startPoint: .top, endPoint: .bottom
    )

    init(kind: String) {
        switch kind {
        case "claude":
            self.init(logo: "agent-claude", tile: Color(red: 0.85, green: 0.47, blue: 0.34).gradient, markScale: 0.56)
        case "cursor":
            self.init(logo: "agent-cursor", tile: Self.night, markScale: 0.52)
        case "codex":
            // As its app icon: a white tile with the gradient cloud, not a blue tile with a white one.
            self.init(
                logo: "agent-codex",
                tile: Color.white,
                markPaint: AnyShapeStyle(Self.codexCloud),
                hairline: Color.black.opacity(0.10),
                markScale: 0.68
            )
        case "grok":
            self.init(logo: "agent-grok", tile: Color(red: 0.03, green: 0.03, blue: 0.03).gradient, markScale: 0.52)
        case "pi":
            self.init(logo: "agent-pi", tile: Self.night, tintsLogo: false, markScale: 0.48)
        case "antigravity":
            // A white tile with the arch in its own colors, like Google's icon.
            self.init(
                logo: "agent-antigravity",
                tile: Color.white,
                tintsLogo: false,
                hairline: Color.black.opacity(0.10),
                markScale: 0.62
            )
        default:
            self.init(logo: nil, tile: Color.gray.gradient, markScale: 0.5)
        }
    }

    private init<S: ShapeStyle>(
        logo: String?,
        tile: S,
        tintsLogo: Bool = true,
        markPaint: AnyShapeStyle = AnyShapeStyle(Color.white),
        hairline: Color = Color.white.opacity(0.12),
        markScale: CGFloat
    ) {
        self.logo = logo
        self.tile = AnyShapeStyle(tile)
        self.tintsLogo = tintsLogo
        self.markPaint = markPaint
        self.hairline = hairline
        self.markScale = markScale
    }
}
