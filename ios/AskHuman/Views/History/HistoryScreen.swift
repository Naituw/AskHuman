import SwiftUI
import AHCore
import AHMarkdown
import AHProtocol
import AHStore

struct HistoryScreen: View {
    @Environment(AppStore.self) private var store
    /// The entry that is open: the one pushed over the list on a phone, the one beside it in a wide window.
    /// Both ends read it from here, so a window that changes from one to the other keeps what was open.
    @State private var opened: String?

    var body: some View {
        ColumnsOrStack {
            HistoryColumns(opened: $opened)
        } stack: {
            NavigationStack(path: Binding(get: { opened.map { [$0] } ?? [] }, set: { opened = $0.last })) {
                HistoryList()
                    .awayStrip()
                    .navigationDestination(for: String.self) { id in
                        if let entry = store.history.first(where: { $0.id == id }) {
                            HistoryDetailScreen(entry: entry)
                        } else {
                            ContentUnavailableView("Entry removed", systemImage: "trash")
                        }
                    }
                    .prefersToolbarItemsOverTabBar()
            }
        }
    }
}

/// A wide window: the history on the left, and on the right the entry that was picked from it.
private struct HistoryColumns: View {
    @Environment(AppStore.self) private var store
    @Binding var opened: String?

    var body: some View {
        let entry = opened.flatMap { id in store.history.first { $0.id == id } }
        ListDetailColumns(opened: entry?.id) {
            HistoryList(selection: $opened)
                .awayStrip()
        } detail: {
            if let entry {
                HistoryDetailScreen(entry: entry)
                    // Each entry has its own page: what the one before it showed is not carried over.
                    .id(entry.id)
            } else {
                DetailPlaceholder(
                    title: "Select an entry", symbol: "clock",
                    message: "Pick one from the list to see what was asked and how you answered."
                )
            }
        }
        // An entry that was deleted (or the history that was cleared) is no longer the one that is open.
        .onChange(of: entry == nil, initial: true) { _, gone in
            if gone, opened != nil { opened = nil }
        }
    }
}

private struct HistoryList: View {
    @Environment(AppStore.self) private var store
    /// Set where the list is the left column of two: a row is picked, not pushed.
    var selection: Binding<String?>?
    @State private var query = ""
    @State private var confirmClear = false

    var body: some View {
        let entries = filtered
        List {
            ForEach(entries) { entry in
                row(entry)
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

    @ViewBuilder
    private func row(_ entry: HistoryEntry) -> some View {
        if let selection {
            HistoryRow(entry: entry)
                .pickable(picked: selection.wrappedValue == entry.id, card: .plain) { selection.wrappedValue = entry.id }
        } else {
            NavigationLink(value: entry.id) { HistoryRow(entry: entry) }
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
        if let sessionTitle { parts.append(sessionTitle) }
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

    /// Title of the Agent session that asked, when the Mac told one.
    var sessionTitle: String? { SessionTitle.clean(request.sessionTitle) }

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
            AgentBadge(agent: entry.request.agent, size: 34, spoken: entry.sessionTitle != nil)
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline) {
                    if let title = entry.sessionTitle {
                        // As in the inbox: the session's title leads, the logo tells the Agent.
                        Text(verbatim: title).font(.subheadline.weight(.semibold)).lineLimit(1)
                    } else {
                        Text(entry.request.agent.label).font(.subheadline.weight(.semibold))
                        if !entry.request.project.name.isEmpty {
                            Text("· " + entry.request.project.name).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                    Spacer(minLength: 6)
                    Text(entry.finishedAt, format: .dateTime.month().day().hour().minute())
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Text(entry.headline).font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
                HStack(spacing: 4) {
                    Label(entry.outcome.title(by: entry.by), systemImage: entry.outcome.symbol)
                        .foregroundStyle(entry.outcome.color)
                    // The project is on the first line unless the title took its place.
                    if entry.sessionTitle != nil, !entry.request.project.name.isEmpty {
                        Text(verbatim: "· " + entry.request.project.name).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                .font(.caption)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }
}
