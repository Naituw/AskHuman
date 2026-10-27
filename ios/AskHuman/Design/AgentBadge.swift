import SwiftUI
import AHProtocol

/// Colored badge for an Agent: a symbol on a rounded square.
struct AgentBadge: View {
    let agent: AgentRef
    var size: CGFloat = 36

    var body: some View {
        let style = AgentStyle(kind: agent.kind)
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
                .fill(style.color.gradient)
            Image(systemName: style.symbol)
                .font(.system(size: size * 0.46, weight: .semibold))
                .foregroundStyle(.white)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

struct AgentStyle {
    let symbol: String
    let color: Color

    init(kind: String) {
        switch kind {
        case "claude": self = .init(symbol: "sparkle", color: Color(red: 0.85, green: 0.47, blue: 0.34))
        case "cursor": self = .init(symbol: "cursorarrow.rays", color: Color(red: 0.22, green: 0.22, blue: 0.26))
        case "codex": self = .init(symbol: "chevron.left.forwardslash.chevron.right", color: Color(red: 0.06, green: 0.64, blue: 0.5))
        case "grok": self = .init(symbol: "bolt.fill", color: Color(red: 0.3, green: 0.3, blue: 0.34))
        case "pi": self = .init(symbol: "function", color: Color(red: 0.45, green: 0.35, blue: 0.85))
        default: self = .init(symbol: "terminal.fill", color: .gray)
        }
    }

    private init(symbol: String, color: Color) {
        self.symbol = symbol
        self.color = color
    }
}
