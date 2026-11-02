import SwiftUI
import AHCore

/// The projects a quick todo can go to, a group for each Mac (docs/specs/ios-developer-mode.md §6.2).
struct QuickTodoProjectPicker: View {
    @Environment(AppStore.self) private var store
    /// A project was chosen: the page goes back to the text.
    let chosen: () -> Void

    var body: some View {
        @Bindable var composer = store.developer.quickTodo
        List {
            if case .projectGone(let mac)? = composer.failure {
                Section {
                    Text(QuickTodoFailure.projectGone(mac: mac).text)
                        .foregroundStyle(.red)
                        .accessibilityIdentifier("quick-todo-note")
                }
            }
            ForEach(composer.visibleGroups) { group in
                section(group, composer: composer)
            }
        }
        .listStyle(.insetGrouped)
        // Always showing: the list is reached again from the page, and the system would fold the bar away.
        .searchable(text: $composer.query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search projects")
        .overlay {
            if !composer.query.trimmingCharacters(in: .whitespaces).isEmpty, composer.visibleGroups.isEmpty {
                ContentUnavailableView.search(text: composer.query)
            }
        }
        .navigationTitle("Project")
        .navigationBarTitleDisplayMode(.inline)
        .accessibilityIdentifier("quick-todo-picker")
        .task { await composer.loadProjects() }
        .onDisappear {
            composer.closeProjects()
            composer.query = ""
        }
    }

    @ViewBuilder
    private func section(_ group: QuickTodoGroup, composer: QuickTodoComposer) -> some View {
        Section {
            switch group.state {
            case .loading:
                HStack(spacing: 10) {
                    ProgressView()
                    Text("Loading…").foregroundStyle(.secondary)
                }
                .accessibilityIdentifier("quick-todo-loading-" + group.macId)
            case .projects(let list):
                if list.isEmpty {
                    Text("No projects yet").foregroundStyle(.secondary)
                }
                ForEach(list, id: \.key) { project in
                    Button {
                        composer.choose(project)
                        chosen()
                    } label: {
                        QuickTodoProjectChoice(project: project, isCurrent: isCurrent(project, in: composer))
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("quick-todo-project-" + project.id)
                }
            case .noAnswer:
                HStack {
                    Text("Did not answer").foregroundStyle(.secondary)
                    Spacer()
                    Button("Try again") { Task { await composer.retry(macId: group.macId) } }
                        .accessibilityIdentifier("quick-todo-retry-" + group.macId)
                }
            case .offline, .needsUpdate:
                EmptyView()
            }
        } header: {
            Text(verbatim: group.macName)
        } footer: {
            switch group.state {
            case .offline: Text("Not reachable right now.")
            case .needsUpdate: Text("Update AskHuman on this Mac to add todos from here.")
            default: EmptyView()
            }
        }
    }

    private func isCurrent(_ project: TodoListedProject, in composer: QuickTodoComposer) -> Bool {
        composer.target.map { $0.macId == project.macId && $0.projectId == project.id } ?? false
    }
}

/// One project of the list: the look of the Todos tab, with a check on the one the todos go to now.
private struct QuickTodoProjectChoice: View {
    let project: TodoListedProject
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
                if let repo = project.repo {
                    Text(verbatim: repo).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                } else if project.scope == "cloudOnly" {
                    Text("Only in iCloud").font(.caption).foregroundStyle(.secondary).lineLimit(1)
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
}
