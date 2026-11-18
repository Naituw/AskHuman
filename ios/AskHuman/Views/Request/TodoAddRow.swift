import SwiftUI
import AHCore
import AHProtocol

// The last row of a todo list on a request page: a dashed line that becomes a multi-line input in place, with a send button
// at its end (docs/specs/todo-list-actions.md D6/D7). Sending adds the todo at the end of the queue and leaves the input
// open for the next one; the keyboard stays. A todo added here is text only (no ⚡, no attachments) and is never picked:
// adding is not answering.

struct TodoAddRow: View {
    let picker: RequestTodoPicker
    /// Where the page scrolls to keep the caret on screen.
    @State private var reveal = InputRevealTarget()

    private let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if picker.isAdding { input } else { line }
            if picker.addProblem == .tooLong {
                Text("That todo is too long.")
                    .font(.caption)
                    .foregroundStyle(.red)
                    .padding(.horizontal, 4)
                    .accessibilityIdentifier("todo-add-problem")
            }
        }
    }

    /// "Add todo" as a dashed line, which opens the input.
    private var line: some View {
        Button {
            withAnimation(.snappy(duration: 0.2)) { picker.beginAdd() }
        } label: {
            Label("Add todo", systemImage: "plus")
                .font(.body)
                .foregroundStyle(picker.canWrite ? Color.accentColor : Color.secondary)
                .frame(maxWidth: .infinity, minHeight: 44)
                .overlay {
                    shape.strokeBorder(
                        (picker.canWrite ? Color.accentColor : Color.secondary).opacity(0.5),
                        style: StrokeStyle(lineWidth: 1, dash: [5, 4])
                    )
                }
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!picker.canWrite)
        .accessibilityIdentifier("todo-add-row")
    }

    private var input: some View {
        HStack(alignment: .bottom, spacing: 8) {
            Image(systemName: "plus")
                .font(.body.weight(.semibold))
                .foregroundStyle(Color.accentColor)
                .frame(width: 22, height: 22)
                .padding(.bottom, 11)
                .accessibilityHidden(true)
            CaretTextView(
                placeholder: String(localized: "New todo"),
                text: Binding(
                    get: { picker.addText },
                    set: {
                        picker.addText = $0
                        picker.addTextChanged()
                    }
                ),
                focused: Binding(get: { picker.addFocused }, set: { picker.addFocusChanged($0) }),
                lines: 1...5, identifier: "todo-add-field", revealTarget: reveal
            )
            .padding(.vertical, 11)
            .background(InputRevealAnchor(target: reveal, part: .editor))
            Button {
                send()
            } label: {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.system(size: 30))
                    .foregroundStyle(canSend ? Color.accentColor : Color(.tertiaryLabel))
                    .frame(width: 34, height: 34)
            }
            .buttonStyle(.plain)
            .disabled(!canSend)
            .padding(.bottom, 5)
            .accessibilityLabel("Add")
            .accessibilityIdentifier("todo-add-send")
        }
        .padding(.leading, 12)
        .padding(.trailing, 6)
        .background(shape.fill(Color(.tertiarySystemFill)))
        .overlay(shape.strokeBorder(Color.accentColor.opacity(0.7), lineWidth: 1.5))
    }

    private var canSend: Bool {
        picker.canWrite && !picker.addText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Adds what is written and clears it for the next one. The field is emptied through the responder chain, as the
    /// Todos tab does: an input method's composition would survive the binding being set.
    private func send() {
        let draft = picker.addText
        picker.submitAdd()
        if picker.addText.isEmpty, !draft.isEmpty { FocusedTextInput.clear() }
    }
}

/// The one line that stands for the whole list of a project that has no todos: it opens the add row.
struct TodoEmptyEntry: View {
    let picker: RequestTodoPicker

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                withAnimation(.snappy(duration: 0.25)) { picker.beginAdd() }
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: "plus.circle.fill")
                        .accessibilityHidden(true)
                    Text("Add todo").font(.body.weight(.semibold))
                    Spacer(minLength: 0)
                }
                .foregroundStyle(picker.canWrite ? Color.accentColor : Color.secondary)
                .frame(minHeight: 28)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!picker.canWrite)
            .accessibilityIdentifier("todo-empty-entry")
            if picker.isOffline {
                Text("Todos can be changed when the Mac is reachable.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("todo-offline-note")
            }
        }
        .card()
    }
}
