import SwiftUI
import AHCore
import AHProtocol

/// The Todos tab: the todos of the paired Macs, per project. The source (iCloud, or one Mac) is chosen
/// in the title; a project opens its todo list.
struct TodosScreen: View {
    @Environment(AppStore.self) private var store
    @Binding var showPairing: Bool

    var body: some View {
        ColumnsOrStack {
            TodosColumns(showPairing: $showPairing)
        } stack: {
            stack
        }
    }

    /// The phone's way: the projects, and the one that was opened pushed over them.
    private var stack: some View {
        NavigationStack {
            Group {
                if store.hasPairedMacs {
                    TodosList().awayStrip()
                } else {
                    OnboardingView(showPairing: $showPairing)
                }
            }
            .modifier(TodosBar())
            .navigationDestination(
                item: Binding(
                    get: { store.todos.opened },
                    set: { if $0 == nil { store.todos.close() } }
                )
            ) { target in
                TodoProjectScreen(target: target)
            }
            .prefersToolbarItemsOverTabBar()
        }
    }
}

/// A wide window: the projects on the left, and on the right the todos of the one that was opened.
///
/// What is open is what the store has open (`todos.opened`, as it is for the phone's stack), so a project
/// that goes away with its Mac leaves the right side to the placeholder.
private struct TodosColumns: View {
    @Environment(AppStore.self) private var store
    @Binding var showPairing: Bool

    var body: some View {
        let open = store.todos.opened
        // Before the Macs have answered nobody knows whether one is paired: the list stays until they say
        // so, as it does for everyone who has one.
        let hasList = store.hasPairedMacs || !store.hasState
        ListDetailColumns(hasList: hasList, opened: open.map { "\($0.macId)/\($0.id)" }) {
            TodosList(columns: true)
                .awayStrip()
                .modifier(TodosBar())
        } detail: {
            if !hasList {
                OnboardingView(showPairing: $showPairing)
            } else if let open {
                TodoProjectScreen(target: open)
                    // Each project has its own page: what is typed or folded on one is not carried to the next.
                    .id("\(open.macId)/\(open.id)")
            } else {
                DetailPlaceholder(
                    title: "Select a project", symbol: "checklist",
                    message: "Pick one from the list to see and edit its todos."
                )
            }
        }
    }
}

/// The title in the bar of the list of projects: where the todos come from.
private struct TodosBar: ViewModifier {
    @Environment(AppStore.self) private var store

    func body(content: Content) -> some View {
        content
            .navigationTitle("Todos")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if store.hasPairedMacs {
                    ToolbarItem(placement: .principal) { TodoSourceMenu() }
                }
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
struct ProjectSearch: ViewModifier {
    @Binding var text: String
    let enabled: Bool
    var placement: SearchFieldPlacement = .automatic

    func body(content: Content) -> some View {
        if enabled {
            content.searchable(text: $text, placement: placement, prompt: "Search projects")
        } else {
            content
        }
    }
}

private struct TodosList: View {
    @Environment(AppStore.self) private var store
    /// Set where the list is the left column of two: the project that is open is tinted.
    var columns = false

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
                        let picked = columns && todos.opened?.macId == project.macId && todos.opened?.id == project.id
                        Button {
                            todos.open(project)
                        } label: {
                            TodoProjectRowView(project: project, kind: section.kind, pushes: !columns)
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("todo-project-" + project.id)
                        .pickedRow(columns ? picked : nil)
                    }
                } header: {
                    TodoSectionHeader(
                        section: section, macName: todos.macName(section.macId), emptyExpanded: $todos.emptyExpanded,
                        toggleId: "todo-empty-toggle"
                    )
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

/// The heading of a group of projects, here and in the project choice of the developer mode. The group of
/// projects without todos folds: it stays closed until asked for.
struct TodoSectionHeader: View {
    let section: TodoSection
    /// The Mac of a group of projects that are only on one Mac.
    let macName: String?
    @Binding var emptyExpanded: Bool
    /// What the UI tests call the fold.
    let toggleId: String

    var body: some View {
        if section.kind == .empty {
            Button {
                emptyExpanded.toggle()
            } label: {
                HStack(spacing: 6) {
                    section.title(macName: macName)
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
            .accessibilityIdentifier(toggleId)
            .animation(.snappy, value: section.collapsed)
        } else {
            section.title(macName: macName).font(.footnote.weight(.semibold)).textCase(nil)
        }
    }
}

private struct TodoProjectRowView: View {
    @Environment(AppStore.self) private var store
    let project: TodoListedProject
    let kind: TodoSection.Kind
    /// The row opens its page over the list (the phone's stack): it says so with an arrow.
    var pushes = true

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
            if pushes {
                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
            }
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
