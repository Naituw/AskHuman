import SwiftUI
import AHCore
import AHMarkdown
import AHProtocol

struct InboxRow: View {
    let item: InboxItem
    let hasDraft: Bool
    /// The list is a column beside the request (a wide window): the unread dot is where the arrival animation of a new
    /// request lands, so the animation can find it, and it is held back until the bubble has become it (`ArrivalDots`).
    var landsArrival = false

    @Environment(AppStore.self) private var store
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// The bubble of this request is on its way to the dot.
    private var dotHeld: Bool {
        landsArrival && !reduceMotion && store.requestArrivals.contains { $0.id == item.id }
            && !ArrivalDots.shared.reached.contains(item.id)
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Circle()
                .fill(Color.accentColor)
                .frame(width: 9, height: 9)
                .opacity(item.unread && !dotHeld ? 1 : 0)
                .animation(.easeOut(duration: 0.15), value: dotHeld)
                // Behind the dot, not behind what is held back: the animation measures the marker.
                .background { if landsArrival { ArrivalDotMarker(key: item.id) } }
                .padding(.top, 14)
                .accessibilityHidden(true)
            AgentBadge(agent: item.summary.agent, spoken: item.sessionTitle != nil)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline) {
                    // The session's title names the request; the Agent is told by its logo. Without a title the
                    // Agent's name leads, as before.
                    Text(verbatim: item.displayTitle)
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
            // No flag for a recommended option: that is a property of the option (the page of the request marks
            // it beside it), and the Agents mark one in nearly every question, so on a row it told nothing.
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
