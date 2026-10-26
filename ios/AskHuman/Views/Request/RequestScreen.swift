import SwiftUI
import AHCore
import AHMarkdown
import AHProtocol

/// One request page: the question form, the confirmation page, or what is known while the rest loads.
struct RequestScreen: View {
    let id: String
    @Environment(AppStore.self) private var store

    /// The receipt of this page, while it shows one.
    private var receipt: SentReceipt? {
        store.receipt?.from == id ? store.receipt : nil
    }

    var body: some View {
        Group {
            if let item = store.displayItem(id: id) {
                if item.view?.ask != nil {
                    AnswerScreen(item: item)
                } else if item.view?.confirm != nil {
                    ConfirmScreen(item: item)
                } else {
                    LoadingRequest(item: item)
                }
            } else {
                ContentUnavailableView("Request unavailable", systemImage: "tray", description: Text("It was answered or cancelled."))
            }
        }
        // The receipt takes the form's place for the moment between "the Mac took it" and the next page.
        .accessibilityHidden(receipt != nil)
        .overlay {
            // The fade belongs to the receipt alone: when this page is handed over to another request
            // (the one below it was dropped), the content must change at once, not cross-fade.
            ZStack {
                if let receipt {
                    SentReceiptPage(receipt: receipt, item: store.displayItem(id: id))
                        .transition(.opacity)
                }
            }
            .animation(.easeOut(duration: 0.2), value: receipt?.id)
        }
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .task(id: id) {
            if let item = store.item(id: id) { await store.opened(item) }
        }
    }
}

/// What a page shows once the Mac took the reply: a plain page of its own, so that what follows
/// (the next request sliding in) reads as a new page and not as the same form changing.
private struct SentReceiptPage: View {
    let receipt: SentReceipt
    let item: InboxItem?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var shown = false

    var body: some View {
        ZStack {
            Color(.systemGroupedBackground).ignoresSafeArea()
            VStack(spacing: 16) {
                Image(systemName: receipt.outcome.symbol)
                    .font(.system(size: 72))
                    .foregroundStyle(receipt.outcome.color)
                    .symbolEffect(.bounce, options: .nonRepeating, value: shown)
                    .scaleEffect(shown || reduceMotion ? 1 : 0.5)
                    .opacity(shown || reduceMotion ? 1 : 0)
                VStack(spacing: 6) {
                    title.font(.title2.weight(.semibold))
                    if let item {
                        HStack(spacing: 8) {
                            AgentBadge(agent: item.summary.agent, size: 22)
                            Text(verbatim: item.summary.agent.label + (item.summary.project.name.isEmpty ? "" : " · " + item.summary.project.name))
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                    }
                }
                .opacity(shown || reduceMotion ? 1 : 0)
                .offset(y: shown || reduceMotion ? 0 : 8)
            }
            .padding(.horizontal, 32)
        }
        .onAppear {
            withAnimation(reduceMotion ? nil : .spring(duration: 0.5, bounce: 0.35)) { shown = true }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("sent-receipt")
    }

    private var title: Text {
        switch receipt.outcome {
        case .answered: Text("Sent")
        case .cancelled: Text("Cancelled")
        case .dismissed: Text("Dismissed")
        default: Text(receipt.outcome.title(by: nil))
        }
    }
}

private struct LoadingRequest: View {
    let item: InboxItem

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                RequestHeader(item: item)
                Text(MarkdownParser.plainText(item.summary.preview)).font(.body)
                HStack(spacing: 8) {
                    ProgressView()
                    Text("Loading the full request…").foregroundStyle(.secondary)
                }
            }
            .padding()
        }
        .background(Color(.systemGroupedBackground))
    }
}

/// Who asks, about which project, on which Mac, and when.
struct RequestHeader: View {
    let item: InboxItem

    var body: some View {
        HStack(spacing: 12) {
            AgentBadge(agent: item.summary.agent, size: 44)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.summary.agent.label).font(.headline)
                HStack(spacing: 5) {
                    if !item.summary.project.name.isEmpty {
                        Image(systemName: "folder.fill").imageScale(.small)
                        Text(item.summary.project.name)
                        Text("·")
                    }
                    Image(systemName: "desktopcomputer").imageScale(.small)
                    Text(item.macName)
                }
                .font(.footnote)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                Text(Date(timeIntervalSince1970: TimeInterval(item.summary.createdAt) / 1000), format: .dateTime.month().day().hour().minute())
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            Spacer(minLength: 0)
        }
    }
}

/// The notice on a page whose request ended while it was open.
struct FinishedNotice: View {
    let finished: FinishedRequest

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text("This request has ended").font(.subheadline.weight(.semibold))
                Text(detail).font(.footnote).foregroundStyle(.secondary)
            }
        } icon: {
            Image(systemName: finished.outcome.symbol).foregroundStyle(finished.outcome.color)
        }
        .card()
        .accessibilityIdentifier("finished-notice")
    }

    private var detail: String {
        switch finished.outcome {
        case .answeredElsewhere: finished.by.map { String(localized: "It was answered on \($0).") } ?? String(localized: "It was answered elsewhere.")
        case .cancelled: String(localized: "The Agent cancelled it.")
        case .expired: String(localized: "It timed out.")
        case .answered: String(localized: "It was answered.")
        case .dismissed: String(localized: "It was dismissed.")
        case .ended: String(localized: "It finished while you were away.")
        }
    }
}

/// Shown when the last submission did not get through.
struct FailedSubmissionNotice: View {
    let item: InboxItem
    @Environment(AppStore.self) private var store
    @State private var problem: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Your reply was not delivered", systemImage: "exclamationmark.triangle.fill")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.red)
            Text("The Mac did not confirm it. You can send it again or discard it and edit your answer.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            HStack {
                Button("Send again") {
                    Task {
                        do { try await store.retry(item) } catch { problem = error.userMessage }
                    }
                }
                .buttonStyle(.borderedProminent)
                Button("Discard", role: .destructive) { Task { await store.discardFailure(item) } }
                    .buttonStyle(.bordered)
            }
            if let problem { Text(problem).font(.footnote).foregroundStyle(.red) }
        }
        .card()
    }
}
