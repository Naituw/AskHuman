import SwiftUI
import AHCore
import AHProtocol

/// How a task that was sent is doing: sent, started in a terminal, found as an Agent session.
struct TaskProgressScreen: View {
    let id: String

    @Environment(AppStore.self) private var store
    @State private var problem: String?
    @State private var busy = false

    var body: some View {
        Group {
            if let task = store.task(id: id) {
                content(task)
            } else {
                ContentUnavailableView("This task is gone", systemImage: "checkmark.circle")
            }
        }
        .navigationTitle("Task")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Close") { store.dismissNewTask() }
                    .accessibilityIdentifier("task-close")
            }
        }
        .alert("Could not do that", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(problem ?? "")
        }
        .sensoryFeedback(.success, trigger: store.task(id: id)?.foundAgent == true)
    }

    private func content(_ task: TaskProgress) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                summary(task)
                StageList(task: task)
                outcome(task)
            }
            .padding()
        }
        .background(Color(.systemGroupedBackground))
        .accessibilityIdentifier("task-progress")
    }

    private func summary(_ task: TaskProgress) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(verbatim: task.agentLabel + " · " + task.workspace)
                .font(.subheadline.weight(.semibold))
            // What was picked in the form, not what the Agent reports: the page does not wait for it.
            if task.hasModelPick {
                Text(verbatim: TaskTexts.pick(modelLabel: task.modelLabel, effort: task.effort))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("task-model-line")
            }
            if !task.attachmentNames.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(task.attachmentNames.enumerated()), id: \.offset) { _, name in
                        Label { Text(verbatim: name) } icon: { Image(systemName: "paperclip") }
                    }
                }
                .font(.footnote).foregroundStyle(.secondary)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("task-attachments-summary")
            }
            if let speed = task.speed {
                Text(speed == .fast ? "Selected speed: Fast" : "Selected speed: Standard")
                    .font(.footnote).foregroundStyle(.secondary)
                    .accessibilityIdentifier("task-speed-pick")
            }
            Text(verbatim: task.text)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(4)
        }
        .card()
    }

    @ViewBuilder
    private func outcome(_ task: TaskProgress) -> some View {
        if task.foundAgent, let agentKey = task.agentKey {
            VStack(alignment: .leading, spacing: 10) {
                Label("The Agent is running.", systemImage: "checkmark.circle.fill")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.green)
                // The Mac followed the Agent (`watching`) unless it already had as many as it allows.
                let note: LocalizedStringKey = task.stage == "watching"
                    ? "You get a notification when it finishes or fails, once your Mac has been idle for a while."
                    : "You will not be notified about this one: this Mac already has as many Agents to tell you about as it allows."
                Text(note)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Button {
                    store.openAgent(macId: task.macId, agentKey: agentKey)
                    store.dismissNewTask()
                } label: {
                    Text("Open Agent").frame(maxWidth: .infinity).padding(.vertical, 8)
                }
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("task-open-agent")
            }
            .card()
        } else if task.stage == "unmatched" {
            VStack(alignment: .leading, spacing: 10) {
                Label("The Agent has not reported in", systemImage: "questionmark.circle.fill")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.orange)
                Text("The terminal was opened, but no session showed up within a minute. Look at the terminal window on your Mac. The Agent may be waiting for you there, or it did not start.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            .card()
        } else if task.stage == "failed" || task.stage == "lost" {
            VStack(alignment: .leading, spacing: 10) {
                Label("The task was not started", systemImage: "xmark.octagon.fill")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.red)
                Text(verbatim: task.stage == "lost" ? TaskTexts.failure("connection") : TaskTexts.failure(task.reason))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                HStack(spacing: 10) {
                    if task.canRetry {
                        Button("Try again") { retry(task) }
                            .buttonStyle(.borderedProminent)
                            .disabled(busy)
                            .accessibilityIdentifier("task-retry")
                    }
                    if store.taskComposer != nil {
                        Button("Edit task") { store.viewingTask = nil }
                            .buttonStyle(.bordered)
                            .accessibilityIdentifier("task-edit")
                    }
                    Button("Dismiss", role: .destructive) { dismiss(task) }
                        .buttonStyle(.bordered)
                        .disabled(busy)
                        .accessibilityIdentifier("task-dismiss")
                }
            }
            .card()
        }
    }

    private func retry(_ task: TaskProgress) {
        busy = true
        Task {
            do { try await store.retryTask(task.id) } catch { problem = error.userMessage }
            busy = false
        }
    }

    private func dismiss(_ task: TaskProgress) {
        busy = true
        Task {
            await store.dismissTask(task.id)
            busy = false
            store.dismissNewTask()
        }
    }
}

/// The three things that happen to a task, each with its own state.
private struct StageList: View {
    let task: TaskProgress

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            row("Sent to your Mac", state: sentState)
            row("Starting the Agent in a terminal", state: startState)
            row("Waiting for the session to appear", state: findState)
        }
        .card()
    }

    private enum StepState { case waiting, running, done, failed }

    private var stageRank: Int {
        switch task.stage {
        case "accepted": 1
        case "launching": 2
        case "launched", "unmatched": 3
        case "watching": 4
        default: 0
        }
    }

    private var failed: Bool { task.stage == "failed" || task.stage == "lost" }

    private var sentState: StepState {
        if task.stage == "failed" && task.reason == "connection" || task.stage == "lost" { return .failed }
        return stageRank >= 1 || task.foundAgent ? .done : (failed ? .failed : .running)
    }

    private var startState: StepState {
        if task.foundAgent || stageRank >= 3 { return .done }
        if failed { return sentState == .done ? .failed : .waiting }
        return stageRank >= 1 ? .running : .waiting
    }

    private var findState: StepState {
        if task.foundAgent { return .done }
        if task.stage == "unmatched" { return .failed }
        if failed { return .waiting }
        return stageRank >= 3 ? .running : .waiting
    }

    private func row(_ title: LocalizedStringKey, state: StepState) -> some View {
        HStack(spacing: 12) {
            ZStack {
                switch state {
                case .waiting: Image(systemName: "circle").foregroundStyle(.tertiary)
                case .running: ProgressView().controlSize(.small)
                case .done: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                case .failed: Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
                }
            }
            .frame(width: 24)
            Text(title)
                .font(.subheadline)
                .foregroundStyle(state == .waiting ? Color.secondary : Color.primary)
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .combine)
    }
}
