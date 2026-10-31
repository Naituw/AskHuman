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
                    // Read only: the Mac tells the phone about this Agent's first result.
                    if item.view.followed, !item.isEnded {
                        Image(systemName: "bell.fill")
                            .imageScale(.small)
                            .foregroundStyle(.secondary)
                            .accessibilityLabel("Will notify you")
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
                    if let title = item.resultTitle, !item.isWorking, item.view.state != "waiting" {
                        Chip(text: title, tint: item.resultColor)
                    }
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
                if item.needsAttention, let reason = item.resultReason {
                    Text(verbatim: reason).font(.footnote).foregroundStyle(.secondary).lineLimit(2)
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


/// Both entry points share an operation identity and keep the free-form draft untouched.
struct AgentContinueButton: View {
    let item: AgentItem
    @Environment(AppStore.self) private var store
    @State private var problem: String?
    @State private var notice: String?
    var body: some View {
        let action = store.continuation(for: item)
        VStack(alignment: .leading, spacing: 6) {
            Button {
                Task {
                    do {
                        let outcome = try await store.continueAgent(item)
                        if outcome.notificationRegistered == false { notice = String(localized: "Message sent. The notification limit has been reached.") }
                    } catch { problem = error.userMessage }
                }
            } label: {
                HStack {
                    if action.isSending { ProgressView() }
                    Text(action.isSending ? "Waiting for the Agent to start…" : "Request continuation")
                }
            }
            .buttonStyle(.bordered)
            .disabled(action.isSending || action.submittedResultId == item.view.lastTurnResult?.id || !store.isOnline(macId: item.macId) || !item.view.canInterject.ok)
            .accessibilityIdentifier("agent-continue-\(item.agentKey)")
            if let notice { Text(notice).font(.footnote).foregroundStyle(.secondary) }
        }
        .alert("Could not send", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) {
            if store.needsAgentDeliveryReview(item) { Button("I checked the original chat") { store.confirmAgentDeliveryReviewed(item) } }
            Button("OK", role: .cancel) {}
        } message: { Text(problem ?? "") }
    }
}
