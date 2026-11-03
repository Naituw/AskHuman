import SwiftUI
import AHCore

// The project of a quick todo is chosen in two steps (docs/specs/ios-developer-mode.md §6.2): where to look
// first, then which project. Both lists are the Todos tab's, drawn again: the same sources (iCloud, and the
// Macs that are not part of it), the same groups and the same search.

/// A page of the choice, as the navigation path of the quick-add page names it.
enum QuickTodoStep: Hashable {
    /// The sources: iCloud, and the Macs that are not part of it.
    case sources
    /// The projects of one source.
    case projects(TodoSource)
}

/// The note that says the project is gone, at the top of whichever list opens first after it was found out.
private struct QuickTodoGoneNote: View {
    let failure: QuickTodoFailure?

    var body: some View {
        if let failure, case .projectGone = failure {
            Section {
                Text(failure.text)
                    .foregroundStyle(.red)
                    .accessibilityIdentifier("quick-todo-note")
            }
        }
    }
}

// MARK: sources

/// The first list: iCloud, then each Mac that is not part of it, as the title of the Todos tab offers them.
/// A source that cannot be looked into (a Mac that is offline or too old, iCloud with no Mac that syncs) is
/// listed all the same, dimmed, with the reason under it.
struct QuickTodoSourcePicker: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let composer = store.developer.quickTodo
        List {
            QuickTodoGoneNote(failure: composer.failure)
            if composer.isLearning {
                // What the Macs do with their todos decides what the sources are: they are not shown half-known.
                SkeletonRows(count: 2)
            } else {
                Section {
                    ForEach(composer.sources) { entry in
                        NavigationLink(value: QuickTodoStep.projects(entry.source)) {
                            QuickTodoSourceRow(choice: entry.choice, isCurrent: composer.currentSource == entry.source)
                        }
                        .disabled(!entry.canOpen)
                        .accessibilityIdentifier("quick-todo-source-" + entry.id)
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle("Source")
        .navigationBarTitleDisplayMode(.inline)
        .accessibilityIdentifier("quick-todo-picker")
    }
}

private struct QuickTodoSourceRow: View {
    let choice: TodoSourceChoice
    /// The project the todos go to now is in this source.
    let isCurrent: Bool

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: choice.symbol)
                .font(.title3)
                .foregroundStyle(choice.source == .icloud ? Color.accentColor : Color.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 2) {
                choice.titleText.font(.body.weight(.medium)).lineLimit(1)
                if let subtitle = choice.subtitleText {
                    subtitle.font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
            }
            Spacer(minLength: 8)
            if isCurrent {
                Image(systemName: "checkmark").font(.body.weight(.semibold)).foregroundStyle(Color.accentColor)
            }
        }
        .padding(.vertical, 2)
    }
}

// MARK: projects

/// The second list: the projects of one source, grouped and searched as on the Todos tab.
struct QuickTodoProjectList: View {
    @Environment(AppStore.self) private var store
    let source: TodoSource
    /// A project was chosen: the page goes back to the text.
    let chosen: () -> Void

    var body: some View {
        @Bindable var composer = store.developer.quickTodo
        // A Mac that turned out to be part of iCloud has no list of its own any more: iCloud's is what it lists.
        let entry = composer.sources.first { $0.source == source } ?? composer.sources.first { $0.source == .icloud }
        let listed = entry?.source ?? source
        List {
            QuickTodoGoneNote(failure: composer.failure)
            if case .loading? = entry?.state {
                SkeletonRows()
            }
            ForEach(composer.sections(of: listed)) { section in
                Section {
                    // Two Macs can send the same row id (it is a hash of a path): rows are told apart by Mac too.
                    ForEach(section.rows, id: \.key) { project in
                        Button {
                            composer.choose(project, in: listed)
                            chosen()
                        } label: {
                            QuickTodoProjectChoice(
                                project: project, kind: section.kind, source: listed,
                                isCurrent: composer.isCurrent(project, in: listed)
                            )
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("quick-todo-project-" + project.id)
                    }
                } header: {
                    TodoSectionHeader(
                        section: section, macName: composer.macName(section.macId), emptyExpanded: $composer.emptyExpanded,
                        toggleId: "quick-todo-empty-toggle"
                    )
                }
            }
        }
        .listStyle(.insetGrouped)
        // Always showing: the list is reached again from the page, and the system would fold the bar away.
        .modifier(ProjectSearch(text: $composer.query, enabled: !isFailed(entry), placement: .navigationBarDrawer(displayMode: .always)))
        .emptyState { QuickTodoProjectsEmptyState(entry: entry, source: listed) }
        .refreshable { await composer.retry(listed) }
        .navigationTitle(entry?.choice.titleText ?? Text("Source"))
        .navigationBarTitleDisplayMode(.inline)
        .accessibilityIdentifier("quick-todo-projects")
        .onDisappear {
            // The next source starts with a clean search and its "without todos" group shut.
            composer.query = ""
            composer.emptyExpanded = false
        }
    }

    private func isFailed(_ entry: QuickTodoSource?) -> Bool {
        if case .failed? = entry?.state { return true }
        return false
    }
}

/// What the list says when it has nothing to list: why the source cannot be shown, or that nothing is there.
private struct QuickTodoProjectsEmptyState: View {
    @Environment(AppStore.self) private var store
    let entry: QuickTodoSource?
    let source: TodoSource

    var body: some View {
        let composer = store.developer.quickTodo
        switch entry?.state {
        case .failed(let failure)?:
            let texts = TodoFailureTexts(failure, name: entry?.choice.title)
            ContentUnavailableView {
                Label(texts.title, systemImage: texts.symbol)
            } description: {
                texts.detail
            } actions: {
                if texts.canRetry {
                    Button("Try again") { Task { await composer.retry(source) } }
                        .buttonStyle(.bordered)
                        .accessibilityIdentifier("quick-todo-retry")
                }
            }
        case .projects?:
            if composer.sections(of: source).isEmpty {
                if composer.query.trimmingCharacters(in: .whitespaces).isEmpty {
                    ContentUnavailableView(
                        "No projects yet", systemImage: "checklist",
                        description: Text("Projects show up here once an Agent has worked in them on a Mac.")
                    )
                } else {
                    ContentUnavailableView.search(text: composer.query)
                }
            }
        case .loading?, nil:
            EmptyView()
        }
    }
}

/// One project of the list: the look of the Todos tab, with a check on the one the todos go to now.
private struct QuickTodoProjectChoice: View {
    let project: TodoListedProject
    let kind: TodoSection.Kind
    let source: TodoSource
    let isCurrent: Bool

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: symbol)
                .font(.title3)
                .foregroundStyle(project.scope == "cloudOnly" ? Color.accentColor : Color.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(verbatim: project.name).font(.body.weight(.medium)).lineLimit(1)
                    if let hint = project.hint {
                        Text(verbatim: hint).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                if let subtitle {
                    subtitle.font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            if project.pending > 0 {
                Text("\(project.pending)")
                    .font(.subheadline.weight(.semibold).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 2)
                    .background(Color(.tertiarySystemFill), in: Capsule())
            }
            if isCurrent {
                Image(systemName: "checkmark").font(.body.weight(.semibold)).foregroundStyle(Color.accentColor)
            }
        }
        .contentShape(Rectangle())
        .padding(.vertical, 2)
    }

    private var symbol: String {
        switch project.scope {
        case "cloudOnly": "icloud"
        case "synced": "folder.badge.gearshape"
        default: "folder"
        }
    }

    private var subtitle: Text? {
        if let repo = project.repo { return Text(verbatim: repo) }
        if project.scope == "cloudOnly" { return Text("Only in iCloud") }
        // The projects without todos of every Mac share one group: each says which Mac it is on.
        if kind == .empty, source == .icloud, project.isOnlyOnItsMac, !project.macName.isEmpty {
            return Text("Only on \(project.macName)")
        }
        return nil
    }
}
