import SwiftUI
import AHCore
import AHProtocol

/// The Todos tab: the todos of the paired Macs, per project. The source (iCloud, or one Mac) is chosen
/// in the title; a project opens its todo list.
struct TodosScreen: View {
    @Environment(AppStore.self) private var store
    @Binding var showPairing: Bool

    var body: some View {
        NavigationStack {
            Group {
                if store.hasPairedMacs {
                    TodosList()
                } else {
                    OnboardingView(showPairing: $showPairing)
                }
            }
            .navigationTitle("Todos")
            .navigationBarTitleDisplayMode(.inline)
            .navigationDestination(
                item: Binding(
                    get: { store.todos.opened },
                    set: { if $0 == nil { store.todos.close() } }
                )
            ) { target in
                TodoProjectScreen(target: target)
            }
            .toolbar {
                if store.hasPairedMacs {
                    ToolbarItem(placement: .principal) { TodoSourceMenu() }
                }
            }
        }
        // The tab asks the Macs for nothing while it is not the one on screen.
        .onChange(of: store.tab, initial: true) { _, tab in
            store.todos.setVisible(tab == .todos)
        }
    }
}

// MARK: source

/// The title of the tab: where the todos come from, and the way to change it.
private struct TodoSourceMenu: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let todos = store.todos
        let current = todos.choices.first { $0.source == todos.source }
        Menu {
            ForEach(todos.choices) { choice in
                Button {
                    todos.select(choice.source)
                } label: {
                    // A menu item reads its first text as the title and its second as the subtitle.
                    choice.titleText
                    if let subtitle = choice.subtitleText { subtitle }
                    Image(systemName: choice.source == todos.source ? "checkmark" : choice.symbol)
                }
                .disabled(!choice.isChoosable)
                .accessibilityIdentifier("todo-source-" + choice.id)
            }
        } label: {
            HStack(spacing: 5) {
                Image(systemName: current?.symbol ?? "checklist").imageScale(.small)
                if let current { current.titleText.font(.headline).lineLimit(1) } else { Text("Todos").font(.headline) }
                Image(systemName: "chevron.down").font(.caption2.weight(.bold)).foregroundStyle(.secondary)
            }
            .foregroundStyle(.primary)
        }
        .accessibilityLabel("Source")
        .accessibilityIdentifier("todo-source-menu")
    }
}

// MARK: projects

/// The search field, except where there is nothing to search: a Mac that cannot be reached or needs an
/// update leaves an explanation on the page, and a search field above it only gets in the way.
private struct ProjectSearch: ViewModifier {
    @Binding var text: String
    let enabled: Bool

    func body(content: Content) -> some View {
        if enabled {
            content.searchable(text: $text, prompt: "Search projects")
        } else {
            content
        }
    }
}

private struct TodosList: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        @Bindable var todos = store.todos
        let sections = todos.sections
        List {
            TodoSyncLineRow()
            TodoStaleNotice()
            if todos.projects == nil, todos.projectsState == .loading || todos.projectsState == .idle {
                SkeletonRows()
            }
            ForEach(sections) { section in
                Section {
                    // Two Macs can send the same row id (it is a hash of a path): rows are told apart by Mac too.
                    ForEach(section.rows, id: \.key) { project in
                        Button {
                            todos.open(project)
                        } label: {
                            TodoProjectRowView(project: project, kind: section.kind)
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("todo-project-" + project.id)
                    }
                } header: {
                    TodoSectionHeader(section: section)
                }
            }
            if todos.projects?.truncated == true {
                Text("Only the most recent projects are listed.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
        .listStyle(.insetGrouped)
        .modifier(ProjectSearch(text: $todos.query, enabled: todos.projects != nil || !todos.projectsState.isFailed))
        .emptyState { TodosEmptyState() }
        .refreshable { await todos.refresh() }
        .accessibilityIdentifier("todo-projects")
    }
}

private struct TodoSectionHeader: View {
    @Environment(AppStore.self) private var store
    let section: TodoSection

    var body: some View {
        let todos = store.todos
        if section.kind == .empty {
            // Projects without todos stay closed until asked for.
            Button {
                todos.emptyExpanded.toggle()
            } label: {
                HStack(spacing: 6) {
                    title
                    Text("\(section.total)").foregroundStyle(.secondary)
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.bold))
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(section.collapsed ? 0 : 90))
                    Spacer(minLength: 0)
                }
                .font(.footnote.weight(.semibold))
                .textCase(nil)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("todo-empty-toggle")
            .animation(.snappy, value: section.collapsed)
        } else {
            title.font(.footnote.weight(.semibold)).textCase(nil)
        }
    }

    private var title: Text {
        switch section.kind {
        case .withTodos: Text("With todos")
        case .synced: Text("Synced with iCloud")
        case .recent: Text("Recent projects")
        case .empty: Text("Without todos")
        case .localOnly: Text("Only on \(store.todos.macName(section.macId) ?? String(localized: "this Mac"))")
        }
    }
}

private struct TodoProjectRowView: View {
    @Environment(AppStore.self) private var store
    let project: TodoListedProject
    let kind: TodoSection.Kind

    private var row: TodoProjectRow { project.row }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: symbol)
                .font(.title3)
                .foregroundStyle(row.scope == "cloudOnly" ? Color.accentColor : Color.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(verbatim: row.name).font(.body.weight(.medium)).lineLimit(1)
                    if let hint = row.hint {
                        Text(verbatim: hint).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                if let subtitle {
                    subtitle.font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            // The states of the Agents tab, laid over the project: the Mac that has the Agent does not matter.
            if let activity = store.todos.activity(of: project) {
                StatusDot(color: activity.color).accessibilityLabel(activity.label)
            }
            if row.pending > 0 {
                Text("\(row.pending)")
                    .font(.subheadline.weight(.semibold).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 2)
                    .background(Color(.tertiarySystemFill), in: Capsule())
            }
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }
        .contentShape(Rectangle())
        .padding(.vertical, 2)
    }

    private var symbol: String {
        switch row.scope {
        case "cloudOnly": "icloud"
        case "synced": "folder.badge.gearshape"
        default: "folder"
        }
    }

    private var subtitle: Text? {
        if let repo = row.repo { return Text(verbatim: repo) }
        if row.scope == "cloudOnly" { return Text("Only in iCloud") }
        // The projects without todos of every Mac share one group: each says which Mac it is on.
        if kind == .empty, store.todos.source == .icloud, project.isOnlyOnItsMac, !project.macName.isEmpty {
            return Text("Only on \(project.macName)")
        }
        return nil
    }
}

// MARK: status

/// The line about iCloud sync, for a source whose Mac syncs.
private struct TodoSyncLineRow: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let todos = store.todos
        if let line = todos.syncLine {
            Section {
                HStack(spacing: 10) {
                    Image(systemName: line.symbol)
                        .foregroundStyle(line.isProblem ? Color.red : Color.secondary)
                    VStack(alignment: .leading, spacing: 1) {
                        line.text
                            .font(.subheadline)
                            .foregroundStyle(line.isProblem ? Color.red : Color.primary)
                        if todos.source == .icloud, let via = todos.accessMacName {
                            Text("via \(via)").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("todo-sync-line")
            }
        }
    }
}

/// What a Mac that went away leaves behind: its last rows stay, with a word about it.
private struct TodoStaleNotice: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let todos = store.todos
        if todos.projects != nil, case .failed(let failure) = todos.projectsState {
            Section {
                NoticeRow(id: "todo-stale", text: text(failure, name: todos.sourceTitle ?? todos.accessMacName), symbol: "desktopcomputer.trianglebadge.exclamationmark", tint: .secondary)
            }
        }
    }

    private func text(_ failure: TodoFailure, name: String?) -> LocalizedStringKey {
        let who = name ?? String(localized: "Your Mac")
        switch failure {
        case .timedOut: return "The Mac did not answer. This is the last state it reported."
        default: return "\(who) is not reachable. This is the last state it reported."
        }
    }
}

private struct TodosEmptyState: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let todos = store.todos
        if todos.projects == nil, case .failed(let failure) = todos.projectsState {
            let texts = TodoFailureTexts(failure, name: todos.sourceTitle)
            ContentUnavailableView {
                Label(texts.title, systemImage: texts.symbol)
            } description: {
                texts.detail
            } actions: {
                if texts.canRetry {
                    Button("Try again") { Task { await todos.refresh() } }
                        .buttonStyle(.bordered)
                }
            }
        } else if todos.projectsState == .loaded, todos.sections.isEmpty {
            if todos.query.trimmingCharacters(in: .whitespaces).isEmpty {
                ContentUnavailableView(
                    "No projects yet", systemImage: "checklist",
                    description: Text("Projects show up here once an Agent has worked in them on a Mac.")
                )
            } else {
                ContentUnavailableView.search(text: todos.query)
            }
        }
    }
}
