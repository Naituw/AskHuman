import SwiftUI
import AHCore
import AHMarkdown
import AHProtocol
import AHStore

struct HistoryScreen: View {
    @Environment(AppStore.self) private var store
    @State private var query = ""
    @State private var confirmClear = false

    var body: some View {
        NavigationStack {
            let entries = filtered
            List {
                ForEach(entries) { entry in
                    NavigationLink(value: entry.id) { HistoryRow(entry: entry) }
                        .swipeActions {
                            Button("Delete", role: .destructive) { Task { await store.deleteHistory(id: entry.id) } }
                        }
                        .accessibilityIdentifier("history-row-\(entry.request.id)")
                }
            }
            .listStyle(.plain)
            .overlay {
                if store.history.isEmpty {
                    ContentUnavailableView("No history yet", systemImage: "clock", description: Text("Requests you answer or cancel are listed here. They stay on this phone only."))
                } else if entries.isEmpty {
                    ContentUnavailableView.search(text: query)
                }
            }
            .searchable(text: $query, prompt: "Search history")
            .navigationTitle("History")
            .navigationDestination(for: String.self) { id in
                if let entry = store.history.first(where: { $0.id == id }) {
                    HistoryDetailScreen(entry: entry)
                } else {
                    ContentUnavailableView("Entry removed", systemImage: "trash")
                }
            }
            .toolbar {
                if !store.history.isEmpty {
                    ToolbarItem(placement: .topBarTrailing) {
                        Menu {
                            Button("Clear history", systemImage: "trash", role: .destructive) { confirmClear = true }
                        } label: {
                            Image(systemName: "ellipsis.circle")
                        }
                    }
                }
            }
            .confirmationDialog("Clear the whole history?", isPresented: $confirmClear, titleVisibility: .visible) {
                Button("Clear history", role: .destructive) { Task { await store.clearHistory() } }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("This cannot be undone. Nothing is removed from your Macs.")
            }
        }
    }

    private var filtered: [HistoryEntry] {
        let text = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return store.history }
        return store.history.filter { $0.searchText.localizedCaseInsensitiveContains(text) }
    }
}

extension HistoryEntry {
    /// Everything a search should look through.
    var searchText: String {
        var parts = [request.agent.label, request.project.name, macName]
        if let ask {
            parts.append(ask.message.text)
            for question in ask.questions {
                parts.append(question.text)
                parts.append(contentsOf: question.options.map(\.text))
            }
        }
        if let confirm = request.confirm {
            parts.append(confirm.title)
            parts.append(confirm.detail.summary)
        }
        parts.append(contentsOf: answers.compactMap(\.text))
        if let confirmChoice { parts.append(confirmChoice) }
        if let confirmInput { parts.append(confirmInput) }
        return parts.joined(separator: "\n")
    }

    var ask: AskView? { request.ask }

    var headline: String {
        if let ask {
            let message = ask.message.text.trimmingCharacters(in: .whitespacesAndNewlines)
            let source = message.isEmpty ? (ask.questions.first?.text ?? "") : message
            return MarkdownParser.plainText(source)
        }
        return request.confirm?.title ?? ""
    }
}

private struct HistoryRow: View {
    let entry: HistoryEntry

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            AgentBadge(agent: entry.request.agent, size: 34)
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline) {
                    Text(entry.request.agent.label).font(.subheadline.weight(.semibold))
                    if !entry.request.project.name.isEmpty {
                        Text("· " + entry.request.project.name).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer(minLength: 6)
                    Text(entry.finishedAt, format: .dateTime.month().day().hour().minute())
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Text(entry.headline).font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
                Label(entry.outcome.title(by: entry.by), systemImage: entry.outcome.symbol)
                    .font(.caption)
                    .foregroundStyle(entry.outcome.color)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }
}
