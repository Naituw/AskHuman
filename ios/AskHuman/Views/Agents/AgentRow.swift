import SwiftUI
import AHCore
import AHMarkdown
import AHProtocol

struct AgentRow: View {
    let item: AgentItem
    let showMac: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            AgentBadge(agent: item.badgeAgent)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(verbatim: item.displayTitle)
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    if item.view.followed {
                        Image(systemName: "star.fill")
                            .imageScale(.small)
                            .foregroundStyle(.yellow)
                            .accessibilityLabel("Following")
                    }
                    RelativeTimeText(date: item.lastActivity)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Text(verbatim: subtitle)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if item.view.state == "waiting", let preview = item.view.waitingPreview, !preview.isEmpty {
                    Text(verbatim: MarkdownParser.plainText(preview))
                        .font(.subheadline)
                        .lineLimit(2)
                }
                FlowLayout(spacing: 6, lineSpacing: 4) {
                    Chip(text: item.stateTitle, tint: item.stateColor)
                    if item.view.state == "working" {
                        if let tool = item.view.currentTool, !tool.isEmpty {
                            // The tool may carry a whole command: the chip shortens it to the row.
                            Chip(verbatim: tool, systemImage: "wrench.and.screwdriver")
                        }
                        if item.view.turnSteps > 0 {
                            Chip(text: "\(item.view.turnSteps) steps", systemImage: "list.bullet")
                        }
                    }
                    if item.view.pendingInterject > 0 {
                        Chip(text: "\(item.view.pendingInterject) queued", systemImage: "paperplane", tint: .blue)
                    }
                }
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }

    private var subtitle: String {
        var parts = [item.view.project.name.isEmpty ? item.view.kindLabel : item.view.project.name]
        if showMac { parts.append(item.macName) }
        return parts.joined(separator: " · ")
    }
}
