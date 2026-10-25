import SwiftUI
import AHCore
import AHMarkdown
import AHProtocol

struct InboxRow: View {
    let item: InboxItem
    let hasDraft: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Circle()
                .fill(Color.accentColor)
                .frame(width: 9, height: 9)
                .padding(.top, 14)
                .opacity(item.unread ? 1 : 0)
                .accessibilityHidden(true)
            AgentBadge(agent: item.summary.agent)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline) {
                    Text(item.summary.agent.label)
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    RelativeTimeText(date: item.receivedAt)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Text(MarkdownParser.plainText(item.summary.preview))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                statusLine
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private var statusLine: some View {
        switch item.state {
        case .sending:
            HStack(spacing: 6) {
                ProgressView().controlSize(.mini)
                Text("Sending…").font(.caption).foregroundStyle(.secondary)
            }
        case .failed:
            Chip(text: "Not sent — tap to retry", systemImage: "exclamationmark.circle.fill", tint: .red)
        case .pending:
            let chips = flags
            if !chips.isEmpty {
                FlowLayout(spacing: 6, lineSpacing: 4) {
                    ForEach(chips, id: \.id) { Chip(text: $0.text, systemImage: $0.symbol, tint: $0.tint) }
                }
            }
        }
    }

    private struct Flag {
        var id: String
        var text: LocalizedStringKey
        var symbol: String
        var tint: Color
    }

    private var flags: [Flag] {
        var result: [Flag] = []
        if let confirm = item.view?.confirm {
            result.append(Flag(id: "confirm", text: "Confirmation", symbol: "checkmark.shield", tint: .orange))
            _ = confirm
        }
        if let ask = item.view?.ask {
            if ask.questions.count > 1 {
                result.append(Flag(id: "count", text: "\(ask.questions.count) questions", symbol: "list.number", tint: .secondary))
            }
            if ask.selectOnly {
                result.append(Flag(id: "selectOnly", text: "Choose only", symbol: "checklist", tint: .secondary))
            }
            if ask.questions.contains(where: { $0.options.contains(where: \.recommended) }) {
                result.append(Flag(id: "recommended", text: "Recommended", symbol: "star.fill", tint: .yellow))
            }
            if !ask.message.files.isEmpty {
                result.append(Flag(id: "files", text: "\(ask.message.files.count) files", symbol: "paperclip", tint: .secondary))
            }
        }
        if hasDraft {
            result.append(Flag(id: "draft", text: "Draft", symbol: "pencil.line", tint: .blue))
        }
        return result
    }
}
