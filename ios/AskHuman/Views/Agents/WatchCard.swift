import SwiftUI
import AHCore
import AHProtocol

/// The live view of one Agent session: its state, its latest words, what it just did and its TODO list.
struct WatchCard: View {
    let item: AgentItem
    let watch: WatchState?
    /// The Mac is reachable and still lists the session, so frames can arrive.
    let live: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            if let frame = watch?.frame {
                if let text = frame.text, !text.isEmpty {
                    MarkdownView(text: text)
                }
                if !frame.todos.isEmpty {
                    TodoList(todos: frame.todos)
                }
                if !frame.steps.isEmpty || frame.stepsOmitted > 0 {
                    StepTimeline(steps: frame.steps, omitted: frame.stepsOmitted)
                }
                if (frame.text ?? "").isEmpty, frame.todos.isEmpty, frame.steps.isEmpty {
                    Text("Nothing to show yet.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            } else if let closed = watch?.closed {
                ClosedNotice(reason: closed)
            } else if live {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Loading the live view…").font(.subheadline).foregroundStyle(.secondary)
                }
            }
            if let closed = watch?.closed, watch?.frame != nil {
                ClosedNotice(reason: closed)
            }
        }
        .card()
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("watch-card")
    }

    private var header: some View {
        HStack(spacing: 8) {
            StatusDot(color: item.stateColor)
            Text(item.stateTitle).font(.subheadline.weight(.semibold))
            if let elapsed = elapsedSeconds {
                Text("·").foregroundStyle(.secondary)
                Label {
                    ElapsedText(base: elapsed, since: watch?.receivedAt ?? .now, ticking: item.isWorking && live && watch?.closed == nil)
                } icon: {
                    Image(systemName: "timer")
                }
                .font(.subheadline)
                .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            if live, watch?.closed == nil, let frame = watch?.frame, frame.phase != "ended" {
                HStack(spacing: 4) {
                    Circle().fill(Color.red).frame(width: 6, height: 6)
                    Text("Live")
                }
                .font(.caption2.weight(.bold))
                .foregroundStyle(.red)
                .padding(.horizontal, 7)
                .padding(.vertical, 3)
                .background(Color.red.opacity(0.12), in: Capsule())
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("watch-live")
            }
        }
    }

    private var elapsedSeconds: TimeInterval? {
        let secs = watch?.frame?.activeElapsedSecs ?? (item.view.activeElapsedSecs > 0 ? item.view.activeElapsedSecs : nil)
        guard let secs, secs > 0 else { return nil }
        return TimeInterval(secs)
    }
}

/// "3 min, 12 sec", counting on while the Agent works.
private struct ElapsedText: View {
    let base: TimeInterval
    let since: Date
    let ticking: Bool

    var body: some View {
        if ticking {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                Text(verbatim: (base + max(0, context.date.timeIntervalSince(since))).elapsedText)
            }
        } else {
            Text(verbatim: base.elapsedText)
        }
    }
}

private struct ClosedNotice: View {
    let reason: String

    var body: some View {
        Label {
            switch reason {
            case "ended": Text("The session has ended.")
            case "limit": Text("Only a few sessions can be watched live at once. Close another one to watch this.")
            default: Text("The Mac no longer has this session.")
            }
        } icon: {
            Image(systemName: reason == "limit" ? "eye.slash" : "checkmark.circle")
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
    }
}

// MARK: steps

private struct StepTimeline: View {
    let steps: [WatchStep]
    let omitted: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Recent steps").font(.footnote.weight(.semibold)).foregroundStyle(.secondary)
            if omitted > 0 {
                Text("\(omitted) earlier steps not shown")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            ForEach(Array(steps.enumerated()), id: \.offset) { _, step in
                StepRow(step: step)
            }
        }
    }
}

private struct StepRow: View {
    let step: WatchStep

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            ZStack {
                if step.state == "running" {
                    ProgressView().controlSize(.mini)
                } else {
                    Image(systemName: step.state == "failed" ? "exclamationmark.triangle.fill" : icon)
                        .foregroundStyle(step.state == "failed" ? Color.red : Color.secondary)
                }
            }
            .font(.caption)
            .frame(width: 18)
            VStack(alignment: .leading, spacing: 1) {
                label.font(.subheadline.weight(.medium))
                if let object = step.object, !object.isEmpty {
                    Text(verbatim: object)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .truncationMode(.middle)
                }
            }
        }
        .accessibilityElement(children: .combine)
    }

    private var icon: String {
        switch step.kind {
        case "run": "terminal"
        case "read": "doc.text"
        case "write": "pencil"
        default: "wrench.and.screwdriver"
        }
    }

    private var label: Text {
        switch step.kind {
        case "run": Text("Ran")
        case "read": Text("Read")
        case "write": Text("Edited")
        default: Text(verbatim: step.name ?? "")
        }
    }
}

// MARK: todos

private struct TodoList: View {
    let todos: [WatchTodo]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Plan").font(.footnote.weight(.semibold)).foregroundStyle(.secondary)
            ForEach(Array(todos.enumerated()), id: \.offset) { _, todo in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Image(systemName: symbol(todo.state))
                        .foregroundStyle(todo.state == "inProgress" ? Color.accentColor : Color.secondary)
                        .frame(width: 18)
                    Text(verbatim: todo.content)
                        .font(.subheadline)
                        .strikethrough(todo.state == "completed")
                        .foregroundStyle(todo.state == "completed" ? Color.secondary : Color.primary)
                }
                .accessibilityElement(children: .combine)
            }
        }
    }

    private func symbol(_ state: String) -> String {
        switch state {
        case "completed": "checkmark.circle.fill"
        case "inProgress": "circle.dotted.circle"
        default: "circle"
        }
    }
}
