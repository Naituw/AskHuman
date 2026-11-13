import SwiftUI
import AHCore
import AHProtocol

// The project's todos in the answer form (docs/specs/special-replies.md D28/D29, docs/specs/todo-list-actions.md). They are
// an answer too: what is picked goes to the Agent with the reply, and the Mac takes it off the queue. Rows look like options
// (`OptionRow`) and carry the todo actions (`TodoActionRow`); the list ends with the row that adds one (`TodoAddRow`).

/// The "Todos" card under the last question of an ask: one line until it is opened, then the queue as check boxes and the
/// way to add one. A project without todos is only the line that adds the first (`TodoEmptyEntry`). Shown when the form
/// takes free text (a select-only form cannot pick todos, and so is not given the list).
struct TodoPickCard: View {
    let picker: RequestTodoPicker

    var body: some View {
        if picker.showsEntry {
            TodoEmptyEntry(picker: picker)
        } else {
            card
        }
    }

    private var card: some View {
        VStack(alignment: .leading, spacing: 12) {
            Button {
                withAnimation(.snappy(duration: 0.25)) { picker.isExpanded.toggle() }
            } label: {
                header
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("todo-pick-header")
            .accessibilityValue(picker.isExpanded ? Text("Expanded") : Text("Collapsed"))
            if picker.isExpanded {
                VStack(spacing: 8) {
                    ForEach(Array(picker.allRows.enumerated()), id: \.element.id) { index, row in
                        TodoActionRow(picker: picker, todo: row.todo, key: row.id, index: index, single: false)
                    }
                    TodoAddRow(picker: picker)
                }
                .animation(.snappy(duration: 0.25), value: picker.allRows.map(\.id))
                Text("Picked todos are sent to the Agent with your answer and removed from the list. Swipe right to complete, left to delete.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("todo-pick-footnote")
                if picker.isOffline { TodoOfflineNote() }
            }
        }
        .card()
    }

    private var header: some View {
        HStack(spacing: 10) {
            Image(systemName: "checklist")
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            Text("Todos").font(.body.weight(.semibold))
            if picker.count > 0 { Chip(verbatim: "\(picker.count)") }
            Spacer(minLength: 8)
            if picker.pickedCount > 0 {
                Text("\(picker.pickedCount) selected")
                    .font(.subheadline)
                    .foregroundStyle(Color.accentColor)
                    .accessibilityIdentifier("todo-pick-count")
            }
            Image(systemName: "chevron.down")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.secondary)
                .rotationEffect(.degrees(picker.isExpanded ? 180 : 0))
                .accessibilityHidden(true)
        }
        .frame(minHeight: 28)
        .contentShape(Rectangle())
    }
}

/// What's next and the Stop confirmation: the project's queue among the options, the first few of it and then "Show all", and
/// the row that adds a todo, even while there is none. A todo is the next task like any option is; What's next takes several
/// together, Stop takes one.
struct TodoQueueRows: View {
    let picker: RequestTodoPicker

    var body: some View {
        VStack(spacing: 8) {
            ForEach(Array(picker.rows.enumerated()), id: \.element.id) { index, row in
                TodoActionRow(picker: picker, todo: row.todo, key: row.id, index: index, single: picker.singleSelection)
            }
            if picker.canFold {
                Button {
                    withAnimation(.snappy(duration: 0.25)) { picker.showsAll.toggle() }
                } label: {
                    HStack(spacing: 4) {
                        if picker.showsAll {
                            Text("Show less")
                        } else {
                            Text("Show all \(picker.count) todos")
                        }
                        Image(systemName: picker.showsAll ? "chevron.up" : "chevron.down")
                            .font(.caption.weight(.semibold))
                    }
                    .font(.subheadline)
                    .frame(maxWidth: .infinity, minHeight: 44)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .foregroundStyle(Color.accentColor)
                .accessibilityIdentifier("todo-show-all")
            }
            TodoAddRow(picker: picker)
            if picker.isOffline { TodoOfflineNote() }
        }
        .animation(.snappy(duration: 0.25), value: picker.rows.map(\.id))
    }
}

/// Why the rows do not answer to their gestures: the Mac cannot be asked.
private struct TodoOfflineNote: View {
    var body: some View {
        Text("Todos can be changed when the Mac is reachable.")
            .font(.caption)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 4)
            .accessibilityIdentifier("todo-offline-note")
    }
}

// MARK: notes over the page

/// What a write leaves on the page for a moment: "Completed … · Undo" for a completion, and a few words for a write the Mac
/// refused or did not answer. They float above the bar, as the Todos tab's notes do.
struct TodoNoteCapsules: View {
    let picker: RequestTodoPicker
    @Environment(AppStore.self) private var store

    var body: some View {
        VStack(spacing: 8) {
            if let notice = picker.notice {
                Text(notice.notice.message)
                    .font(.footnote.weight(.medium))
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .shadow(color: .black.opacity(0.1), radius: 4, y: 1)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                    .accessibilityIdentifier("todo-notice")
                    .task(id: notice.id) {
                        try? await Task.sleep(for: store.todoNoteHold)
                        if !Task.isCancelled { picker.dismissNotice(notice.id) }
                    }
            }
            if let done = picker.undo {
                HStack(spacing: 10) {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                        .accessibilityHidden(true)
                    Text("Completed “\(Self.shortened(done.text))”")
                        .font(.subheadline)
                        .lineLimit(1)
                    Button("Undo") { picker.undoComplete() }
                        .font(.subheadline.weight(.semibold))
                        .accessibilityIdentifier("todo-undo-button")
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 9)
                .background(.regularMaterial, in: Capsule())
                .overlay(Capsule().strokeBorder(Color.secondary.opacity(0.25)))
                .shadow(color: .black.opacity(0.12), radius: 5, y: 1)
                .transition(.move(edge: .bottom).combined(with: .opacity))
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("todo-undo")
                .task(id: done.id) {
                    try? await Task.sleep(for: store.todoNoteHold)
                    if !Task.isCancelled { picker.dismissUndo(done.id) }
                }
            }
        }
        .padding(.horizontal, 24)
        .padding(.bottom, 12)
        .animation(.snappy, value: picker.notice?.id)
        .animation(.snappy, value: picker.undo?.id)
    }

    /// The first words of a todo, for a note that has one line.
    private static func shortened(_ text: String) -> String {
        let line = text.split(whereSeparator: \.isNewline).first.map(String.init) ?? text
        return line.count > 24 ? String(line.prefix(24)) + "…" : line
    }
}
