import SwiftUI
import AHCore
import AHMarkdown
import AHProtocol

/// One request page: the question form, the confirmation page, or what is known while the rest loads.
struct RequestScreen: View {
    let id: String
    @Environment(AppStore.self) private var store

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
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .task(id: id) {
            if let item = store.item(id: id) { await store.opened(item) }
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
