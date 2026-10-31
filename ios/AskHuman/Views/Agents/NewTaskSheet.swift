import SwiftUI
import AHCore
import AHProtocol

/// The sheet of the Agents tab: the form of a new task, then how that task is going.
struct NewTaskSheet: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        NavigationStack {
            if let taskId = store.viewingTask {
                TaskProgressScreen(id: taskId)
            } else if let composer = store.taskComposer {
                NewTaskForm(composer: composer)
            }
        }
        .presentationDetents([.large])
        .interactiveDismissDisabled(store.viewingTask == nil && (store.taskComposer?.hasContent ?? false))
    }
}

private extension TaskComposer {
    /// The user wrote something that closing the sheet would lose.
    var hasContent: Bool { !trimmedText.isEmpty }
}

private struct NewTaskForm: View {
    let composer: TaskComposer

    @Environment(AppStore.self) private var store
    @Environment(\.launchOptions) private var launch
    @State private var sending = false
    @State private var problem: String?
    @State private var confirmDiscard = false
    @State private var sentTick = 0
    @State private var textFocused = false

    var body: some View {
        @Bindable var composer = composer
        let state = store.state.taskOptions[composer.macId]
        Form {
            if store.taskMacs.count > 1 {
                Section {
                    Picker("Mac", selection: Binding(get: { composer.macId }, set: { store.selectTaskMac($0) })) {
                        ForEach(store.taskMacs) { mac in Text(verbatim: mac.name).tag(mac.id) }
                    }
                    .accessibilityIdentifier("task-mac")
                }
            }
            switch state {
            case .ready(let options):
                if options.enabled {
                    fields(options)
                } else {
                    blockedSection(options.blocked)
                }
            case .failed(let reason):
                Section {
                    Label(TaskTexts.blocked(reason), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.orange)
                    Button("Try again") { store.reloadTaskOptions() }
                }
            case .loading, nil:
                Section {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("Asking \(macName)…").foregroundStyle(.secondary)
                    }
                }
            }
        }
        .navigationTitle("New task")
        .navigationBarTitleDisplayMode(.inline)
        .scrollDismissesKeyboard(.interactively)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { cancel() }
                    .accessibilityIdentifier("task-cancel")
            }
            ToolbarItem(placement: .confirmationAction) {
                if sending {
                    ProgressView()
                } else {
                    Button("Start") { start() }
                        .fontWeight(.semibold)
                        .disabled(!canStart)
                        .accessibilityIdentifier("task-start")
                }
            }
        }
        .confirmationDialog("Discard this task?", isPresented: $confirmDiscard, titleVisibility: .visible) {
            Button("Discard", role: .destructive) { store.dismissNewTask() }
            Button("Keep editing", role: .cancel) {}
        }
        .alert("Could not start the task", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(problem ?? "")
        }
        .sensoryFeedback(.success, trigger: sentTick)
        .onChange(of: store.state.taskOptions[composer.macId]) { _, _ in store.taskOptionsArrived() }
        .onAppear { store.taskOptionsArrived() }
    }

    private var macName: String {
        store.macs.first { $0.id == composer.macId }?.name ?? ""
    }

    private var canStart: Bool {
        guard case .ready(let options)? = store.state.taskOptions[composer.macId] else { return false }
        return composer.canSubmit(options: options) && !sending
    }

    // MARK: fields

    @ViewBuilder
    private func fields(_ options: TaskOptions) -> some View {
        @Bindable var composer = composer
        Section {
            Picker("Workspace", selection: Binding(get: { composer.workspaceId ?? "" }, set: { store.selectTaskWorkspace($0) })) {
                ForEach(options.workspaces) { workspace in
                    VStack(alignment: .leading, spacing: 1) {
                        HStack(spacing: 4) {
                            if workspace.pinned { Image(systemName: "pin.fill").imageScale(.small).foregroundStyle(.secondary) }
                            Text(verbatim: workspace.label)
                        }
                        Text(verbatim: workspace.subtitle).font(.caption).foregroundStyle(.secondary)
                    }
                    .tag(workspace.id)
                }
            }
            .pickerStyle(.navigationLink)
            .accessibilityIdentifier("task-workspace")
        }

        Section {
            ForEach(options.agents, id: \.kind) { agent in
                AgentChoiceRow(agent: agent, selected: composer.agentKind == agent.kind) {
                    store.selectTaskAgent(agent.kind)
                }
            }
        } header: {
            Text("Agent")
        }

        modelSection(options)

        permissionSection(options)

        Section {
            if let todos = store.state.todos(macId: composer.macId, workspaceId: composer.workspaceId ?? ""), !todos.isEmpty {
                Picker("Start from", selection: $composer.todoId) {
                    Text("A new task I write").tag(String?.none)
                    ForEach(todos) { todo in
                        Text(verbatim: (todo.auto ? "⚡ " : "") + todo.text).lineLimit(2).tag(String?.some(todo.id))
                    }
                }
                .pickerStyle(.navigationLink)
                .accessibilityIdentifier("task-todo")
            }
            CaretTextView(
                placeholder: composer.todoId == nil ? String(localized: "Describe the task") : String(localized: "Anything to add? (optional)"),
                text: $composer.text, focused: $textFocused,
                lines: 4...12, identifier: "task-text"
            )
            .contentShape(Rectangle())
            .simultaneousGesture(TapGesture().onEnded { textFocused = true })
        } header: {
            Text("Task")
        } footer: {
            if composer.charCount > TaskComposer.maxChars - 500 {
                Text("\(composer.charCount) / \(TaskComposer.maxChars)")
                    .foregroundStyle(composer.overLimit ? Color.red : Color.secondary)
                    .monospacedDigit()
            } else {
                Text("The Agent starts in a new terminal window on your Mac.")
            }
        }
    }

    /// The model and the effort: only when the Mac listed models for this Agent. The phone restores its
    /// last submitted choice; "Default" leaves the choice to the Agent's own settings.
    @ViewBuilder
    private func modelSection(_ options: TaskOptions) -> some View {
        if composer.offersModelChoice(in: options) {
            Section {
                let picked = composer.model(in: options)
                NavigationLink {
                    ModelPickerScreen(composer: composer)
                } label: {
                    ModelRow(
                        value: picked?.label ?? String(localized: "Default"),
                        detail: picked == nil
                            ? (composer.defaultModel(in: options)?.label ?? String(localized: "Agent's own setting"))
                            : nil
                    )
                }
                .accessibilityIdentifier("task-model")
                if let choices = composer.effortChoices(in: options) {
                    Picker(
                        "Effort",
                        selection: Binding(get: { composer.effort(in: options) }, set: { composer.effortId = $0 })
                    ) {
                        Text(verbatim: TaskTexts.defaultEffort(choices.defaultLevel)).tag(String?.none)
                        ForEach(choices.levels, id: \.self) { level in
                            Text(verbatim: TaskTexts.effort(level)).tag(String?.some(level))
                        }
                    }
                    .pickerStyle(.menu)
                    .accessibilityIdentifier("task-effort")
                }
            }
        }
    }

    @ViewBuilder
    private func permissionSection(_ options: TaskOptions) -> some View {
        Section {
            switch options.permissionMode {
            case "yolo":
                Label("Your Mac lets new tasks skip permission prompts.", systemImage: "exclamationmark.shield")
                    .foregroundStyle(.orange)
            case "agentDefault":
                Label("The Agent asks before it acts, as its own settings say.", systemImage: "hand.raised")
            default:
                if composer.offersPermissionChoice(in: options) {
                    PermissionChoiceRow(
                        title: "Ask before acting", detail: "The Agent asks you before it runs commands or changes files.",
                        symbol: "hand.raised", tint: .accentColor, selected: composer.permissionChoice == .standard
                    ) { composer.permissionChoice = .standard }
                    .accessibilityIdentifier("task-permission-default")
                    PermissionChoiceRow(
                        title: "Skip permission prompts", detail: "The Agent runs commands and changes files without asking. Use it only for tasks you trust.",
                        symbol: "exclamationmark.shield", tint: .red, selected: composer.permissionChoice == .yolo
                    ) { composer.permissionChoice = .yolo }
                    .accessibilityIdentifier("task-permission-yolo")
                } else {
                    Label("This Agent follows its own permission settings.", systemImage: "hand.raised")
                }
            }
        } header: {
            Text("Permissions")
        } footer: {
            if composer.offersPermissionChoice(in: options), composer.permissionChoice == nil {
                Text("Choose how the Agent may act.")
            }
        }
    }

    private func blockedSection(_ reason: String?) -> some View {
        Section {
            Label(TaskTexts.blocked(reason), systemImage: "exclamationmark.triangle")
                .foregroundStyle(.orange)
        }
    }

    // MARK: actions

    private func cancel() {
        if composer.hasContent { confirmDiscard = true } else { store.dismissNewTask() }
    }

    private func start() {
        guard !sending else { return }
        sending = true
        Task {
            defer { sending = false }
            // Starting an Agent on a Mac is the one thing here that asks who is holding the phone.
            if !launch.mock {
                let allowed = await BiometricGate.confirm(reason: String(localized: "Confirm that it is you before an Agent is started on your Mac."))
                guard allowed else { return }
            }
            do {
                if let id = try await store.createTask() {
                    sentTick += 1
                    store.presentTaskProgress(id)
                }
            } catch {
                problem = error.userMessage
            }
        }
    }
}

// MARK: rows

/// The model row: what is picked on the right, what "Default" stands for underneath while nothing is.
private struct ModelRow: View {
    let value: String
    let detail: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Model")
                if let detail {
                    Text(verbatim: detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            Text(verbatim: value)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }
}

private struct AgentChoiceRow: View {
    let agent: AgentOption
    let selected: Bool
    var choose: () -> Void

    var body: some View {
        Button(action: choose) {
            HStack(spacing: 12) {
                AgentBadge(agent: AgentRef(kind: agent.kind, label: agent.label), size: 32)
                    .opacity(agent.ready ? 1 : 0.4)
                VStack(alignment: .leading, spacing: 1) {
                    Text(verbatim: agent.label).foregroundStyle(agent.ready ? Color.primary : Color.secondary)
                    if !agent.ready, let reason = agent.reason, !reason.isEmpty {
                        Text(verbatim: reason).font(.caption).foregroundStyle(.secondary)
                    } else if !agent.subtitle.isEmpty {
                        Text(verbatim: agent.subtitle).font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 0)
                if selected {
                    Image(systemName: "checkmark").foregroundStyle(Color.accentColor).fontWeight(.semibold)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!agent.ready)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier("task-agent-\(agent.kind)")
    }
}

private struct PermissionChoiceRow: View {
    let title: LocalizedStringKey
    let detail: LocalizedStringKey
    let symbol: String
    let tint: Color
    let selected: Bool
    var choose: () -> Void

    var body: some View {
        Button(action: choose) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: symbol)
                    .foregroundStyle(tint)
                    .frame(width: 24)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).foregroundStyle(Color.primary)
                    Text(detail).font(.caption).foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                    .foregroundStyle(selected ? tint : Color.secondary)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
