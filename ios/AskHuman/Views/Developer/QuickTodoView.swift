import SwiftUI
import AHCore

/// The quick-add page of the developer mode (docs/specs/ios-developer-mode.md §6): where the todo goes, and
/// one sentence. It only draws what `QuickTodoComposer` says and calls `DeveloperTools` for what the buttons do.
struct QuickTodoView: View {
    @Environment(AppStore.self) private var store
    /// The choice of the project, when it is open: the sources, then the projects of one of them.
    @State private var path: [QuickTodoStep] = []
    @FocusState private var focused: Bool

    private var developer: DeveloperTools { store.developer }

    var body: some View {
        @Bindable var composer = developer.quickTodo
        let sending = composer.phase == .sending
        NavigationStack(path: $path) {
            Form {
                Section {
                    Button {
                        openThePicker()
                    } label: {
                        QuickTodoProjectRow(target: composer.target)
                    }
                    .buttonStyle(.plain)
                    .disabled(sending)
                    .accessibilityIdentifier("quick-todo-project")
                }
                Section {
                    TextField("New todo", text: $composer.text, axis: .vertical)
                        .lineLimit(3...10)
                        .focused($focused)
                        .disabled(sending)
                        .accessibilityIdentifier("quick-todo-field")
                } footer: {
                    QuickTodoNote(composer: composer)
                }
            }
            .navigationTitle("Quick todo")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    CancelToolbarButton { developer.dismissQuickTodo() }
                        .disabled(sending)
                        .accessibilityIdentifier("quick-todo-cancel")
                }
                ToolbarItem(placement: .confirmationAction) {
                    if sending {
                        ProgressView().accessibilityIdentifier("quick-todo-sending")
                    } else {
                        ConfirmToolbarButton("Add", systemImage: "arrow.up") { Task { await developer.submitQuickTodo() } }
                            .disabled(!composer.canSend)
                            .accessibilityIdentifier("quick-todo-send")
                    }
                }
            }
            .navigationDestination(for: QuickTodoStep.self) { step in
                switch step {
                case .sources: QuickTodoSourcePicker()
                case .projects(let source): QuickTodoProjectList(source: source) { path = [] }
                }
            }
        }
        // While the Mac is being asked there is no way out: a refusal that came after the page was gone
        // would have nowhere to say so.
        .interactiveDismissDisabled(sending)
        .task {
            if composer.target == nil {
                // The first time there is no project yet, so the page opens on the choice of one.
                var transaction = Transaction()
                transaction.disablesAnimations = true
                withTransaction(transaction) { openThePicker() }
            } else {
                focused = true
            }
        }
        .onChange(of: path.isEmpty) { _, closed in
            if closed {
                composer.closeProjects()
                focused = true
            }
        }
        .onChange(of: composer.failure) { _, failure in
            // The project is gone from that Mac: choosing another is the only way on.
            if case .projectGone = failure { openThePicker() }
        }
        // The page goes (a swipe can close it with the choice open): the Macs are not asked any more.
        .onDisappear { composer.closeProjects() }
    }

    /// Opens the choice of the project. When there is just one source that is no choice, so it opens on its
    /// projects. Otherwise, with a project chosen it opens on the projects of that project's source, so that
    /// another one of it is a tap away (the back button goes to the sources); with none, or when that source
    /// cannot be looked into now, it opens on the sources.
    private func openThePicker() {
        let composer = developer.quickTodo
        // Started first: what the Todos tab knows of the Macs by now is what tells the sources apart.
        composer.startLoadingProjects()
        let sources = composer.sources
        if sources.count == 1, let only = sources.first, only.canOpen {
            path = [.projects(only.source)]
        } else if let current = composer.currentSource, sources.first(where: { $0.source == current })?.canOpen == true {
            path = [.sources, .projects(current)]
        } else {
            path = [.sources]
        }
    }
}

/// The line that says where the todo goes.
private struct QuickTodoProjectRow: View {
    let target: QuickTodoTarget?

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: target == nil ? "folder.badge.questionmark" : "folder")
                .font(.title3)
                .foregroundStyle(.secondary)
                .frame(width: 28)
            if let target {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(verbatim: target.projectName).font(.body.weight(.medium)).lineLimit(1)
                        if let hint = target.projectHint {
                            Text(verbatim: hint).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                    // An iCloud project is iCloud's, whichever Mac it goes through.
                    (target.viaICloud ? Text("iCloud") : Text(verbatim: target.macName))
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            } else {
                Text("Choose a project").foregroundStyle(Color.accentColor)
            }
            Spacer(minLength: 8)
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }
        .contentShape(Rectangle())
    }
}

/// Why the todo was not sent, or cannot be yet. Nothing for an empty text: it needs no explaining.
private struct QuickTodoNote: View {
    let composer: QuickTodoComposer

    var body: some View {
        if let failure = composer.failure {
            Text(failure.text)
                .foregroundStyle(.red)
                .accessibilityIdentifier("quick-todo-note")
        } else if let reason = composer.unavailable {
            Text(reason.text)
                .accessibilityIdentifier("quick-todo-note")
        }
    }
}
