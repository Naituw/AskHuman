import SwiftUI
import AHCore
import AHProtocol

/// One Agent session: live view, the question it waits on, and a way to talk to it.
struct AgentDetailScreen: View {
    let id: String
    @Environment(AppStore.self) private var store

    var body: some View {
        if let item = store.displayAgent(id: id) {
            AgentDetailContent(item: item, gone: store.agent(id: id) == nil)
        } else {
            ContentUnavailableView("Session not found", systemImage: "questionmark.circle", description: Text("The Mac no longer lists this session."))
        }
    }
}

private struct AgentDetailContent: View {
    let item: AgentItem
    /// The Mac no longer lists the session (the page shows its last known state).
    let gone: Bool

    @Environment(AppStore.self) private var store
    @State private var problem: String?
    @State private var notice: String?
    @State private var sentTick = 0

    var body: some View {
        let online = store.isOnline(macId: item.macId)
        let live = online && !gone
        let watch = store.watchState(id: item.id)
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if !online {
                    Notice(symbol: "desktopcomputer.trianglebadge.exclamationmark", text: "\(item.macName) is not reachable. This is the last state it reported.", tint: .gray)
                } else if gone {
                    Notice(symbol: "tray", text: "The Mac no longer lists this session.", tint: .gray)
                }
                AgentHeader(item: item, model: watch?.frame?.model, reasoning: watch?.frame?.reasoning)
                if item.view.state == "waiting", let requestId = item.view.waitingRequestId {
                    WaitingCard(item: item) { store.open(macId: item.macId, requestId: requestId) }
                }
                if item.view.pendingInterject > 0 {
                    PendingCard(item: item) { recalled in
                        // The recalled text goes in front of what is already being written.
                        let composer = store.interjectComposer(for: item)
                        composer.text = composer.trimmedText.isEmpty ? recalled : recalled + "\n\n" + composer.text
                    } onProblem: { problem = $0 }
                }
                WatchCard(item: item, watch: watch, live: live)
            }
            .padding()
            .padding(.bottom, 8)
        }
        .background(Color(.systemGroupedBackground))
        .scrollDismissesKeyboard(.interactively)
        .navigationTitle(item.displayTitle)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            bottomBar(live: live)
        }
        .overlay(alignment: .bottom) {
            if let notice {
                Text(notice)
                    .font(.footnote.weight(.medium))
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(.thinMaterial, in: Capsule())
                    .padding(.bottom, 90)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                    .accessibilityIdentifier("interject-notice")
            }
        }
        .animation(.snappy, value: notice)
        .onChange(of: store.consumed[item.id]) { _, date in
            guard date != nil else { return }
            show(String(localized: "The Agent read your message."))
        }
        .alert("Could not send", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(problem ?? "")
        }
        .sensoryFeedback(.success, trigger: sentTick)
    }

    @ViewBuilder
    private func bottomBar(live: Bool) -> some View {
        if live, item.view.canInterject.ok {
            InterjectBar(item: item, composer: store.interjectComposer(for: item)) { outcome in
                sentTick += 1
                show(outcome.status == "delivered" ? String(localized: "Delivered to the Agent.") : String(localized: "Queued. The Agent reads it at its next step."))
            } onProblem: { message in
                problem = message
            }
        } else {
            let reason = item.view.canInterject.reason ?? (item.isEnded ? "ended" : "notWorking")
            Text(BackendError.rejected(reason).userMessage)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .background(.bar)
                .accessibilityIdentifier("interject-unavailable")
        }
    }

    private func show(_ text: String) {
        notice = text
        Task {
            try? await Task.sleep(for: .seconds(3))
            if notice == text { notice = nil }
        }
    }
}

// MARK: pieces

private struct Notice: View {
    let symbol: String
    let text: LocalizedStringKey
    let tint: Color

    var body: some View {
        Label {
            Text(text).font(.footnote)
        } icon: {
            Image(systemName: symbol)
        }
        .foregroundStyle(tint == .gray ? Color.secondary : tint)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

private struct AgentHeader: View {
    let item: AgentItem
    /// What the session runs with, as the Mac reported it in the live view (the page shows what it has).
    let model: String?
    let reasoning: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 12) {
                AgentBadge(agent: item.badgeAgent, size: 44)
                VStack(alignment: .leading, spacing: 3) {
                    Text(verbatim: item.displayTitle)
                        .font(.headline)
                        .lineLimit(3)
                    Text(verbatim: subtitle)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    Text("Started \(Text(item.startedAt, style: .relative)) ago")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            if let line = TaskTexts.running(model: model, reasoning: reasoning) {
                Label {
                    Text(verbatim: line)
                } icon: {
                    Image(systemName: "cpu")
                }
                .font(.footnote)
                .foregroundStyle(.secondary)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("agent-model")
            }
            // Only a task started from the phone is followed (the Agents tab lists every session anyway).
            // The Mac drops the follow once the first result was dealt with, and the line goes with it.
            if item.view.followed, !item.isEnded {
                Label {
                    Text("You get a notification when this Agent finishes or fails, once your Mac has been idle for a while.")
                } icon: {
                    Image(systemName: "bell.fill")
                }
                .font(.footnote)
                .foregroundStyle(.secondary)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("follow-note")
            }
        }
        .card()
    }

    private var subtitle: String {
        let project = item.view.project.name.isEmpty ? item.view.kindLabel : item.view.project.name
        return [project, item.macName, item.view.kindLabel == project ? nil : item.view.kindLabel].compactMap { $0 }.joined(separator: " · ")
    }
}

/// A working Agent asked a question: the way to answer it.
private struct WaitingCard: View {
    let item: AgentItem
    var open: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Waiting for your answer", systemImage: "questionmark.bubble.fill")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.orange)
            if let preview = item.view.waitingPreview, !preview.isEmpty {
                Text(verbatim: preview)
                    .font(.subheadline)
                    .lineLimit(4)
            }
            Button(action: open) {
                Text("Answer")
                    .font(.subheadline.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
            }
            .buttonStyle(.borderedProminent)
            .tint(.orange)
            .accessibilityIdentifier("answer-waiting")
        }
        .card()
        .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).stroke(Color.orange.opacity(0.5), lineWidth: 1))
    }
}

/// Messages the Agent has not picked up yet: they can be taken back.
private struct PendingCard: View {
    let item: AgentItem
    var onRecalled: (String) -> Void
    var onProblem: (String) -> Void

    @Environment(AppStore.self) private var store
    @State private var busy = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("\(item.view.pendingInterject) queued", systemImage: "paperplane.fill")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.blue)
            Text("The Agent reads them at its next step. Until then you can edit or withdraw them.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            HStack(spacing: 10) {
                Button("Edit") { act(recall: true) }
                    .accessibilityIdentifier("interject-edit")
                Button("Withdraw", role: .destructive) { act(recall: false) }
                    .accessibilityIdentifier("interject-withdraw")
            }
            .buttonStyle(.bordered)
            .disabled(busy)
        }
        .card()
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("interject-pending")
    }

    private func act(recall: Bool) {
        busy = true
        Task {
            do {
                if recall {
                    onRecalled(try await store.recallInterject(item))
                } else {
                    try await store.clearInterject(item)
                }
            } catch {
                onProblem(error.userMessage)
            }
            busy = false
        }
    }
}
