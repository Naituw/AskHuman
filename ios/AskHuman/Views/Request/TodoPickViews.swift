import SwiftUI
import AHCore
import AHProtocol

// The project's todos in the answer form (docs/specs/special-replies.md D28/D29). They are an answer too: what is
// picked goes to the Agent with the reply, and the Mac takes it off the queue. Rows look like options and are
// laid out by `OptionRow`.

/// The folded "Todos" card under the last question of an ask: one line until it is opened, then the queue as
/// check boxes. Shown only when the project has todos and the form takes free text.
struct TodoPickCard: View {
    let picker: RequestTodoPicker

    var body: some View {
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
                    ForEach(Array(picker.todos.enumerated()), id: \.element.id) { index, todo in
                        OptionRow(todo: todo, single: false, selected: picker.isPicked(todo.id)) {
                            picker.toggle(todo.id)
                        }
                        .accessibilityIdentifier("todo-row-\(index)")
                    }
                }
                Text("Picked todos are sent to the Agent with your answer and removed from the list.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("todo-pick-footnote")
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
            Chip(verbatim: "\(picker.count)")
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

/// What's next: the project's queue among the options, the first few of it and then "Show all". A todo is the
/// next task like any option is, so one of them is chosen, with the others.
struct TodoQueueRows: View {
    let picker: RequestTodoPicker

    var body: some View {
        VStack(spacing: 8) {
            ForEach(Array(picker.listed.enumerated()), id: \.element.id) { index, todo in
                OptionRow(todo: todo, single: true, selected: picker.isPicked(todo.id)) {
                    picker.toggle(todo.id)
                }
                .accessibilityIdentifier("todo-row-\(index)")
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
        }
    }
}
