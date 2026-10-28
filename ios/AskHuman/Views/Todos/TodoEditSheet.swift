import SwiftUI
import AHCore
import AHProtocol

/// Edits one todo: its text, whether the Agent runs it by itself, or removes it.
struct TodoEditSheet: View {
    let item: TodoItem
    let canAuto: Bool
    @Environment(AppStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var text: String
    @State private var auto: Bool
    @State private var confirmDelete = false
    @FocusState private var focused: Bool

    init(item: TodoItem, canAuto: Bool) {
        self.item = item
        self.canAuto = canAuto
        _text = State(initialValue: item.text)
        _auto = State(initialValue: item.auto)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Todo", text: $text, axis: .vertical)
                        .lineLimit(3...12)
                        .focused($focused)
                        .accessibilityIdentifier("todo-edit-field")
                } footer: {
                    if count > Wire.todoTextMaxChars - 500 {
                        Text("\(count) / \(Wire.todoTextMaxChars)")
                            .monospacedDigit()
                            .foregroundStyle(count > Wire.todoTextMaxChars ? Color.red : Color.secondary)
                    }
                }
                if canAuto {
                    Section {
                        Toggle(isOn: $auto) {
                            Label("Run automatically", systemImage: "bolt.fill")
                        }
                        .accessibilityIdentifier("todo-edit-auto")
                    } footer: {
                        Text("When on, the Agent runs this todo by itself after it finishes the task before it.")
                    }
                }
                Section {
                    Button("Delete todo", role: .destructive) { confirmDelete = true }
                        .accessibilityIdentifier("todo-edit-delete")
                }
            }
            .navigationTitle("Edit todo")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: save)
                        .disabled(!canSave)
                        .accessibilityIdentifier("todo-edit-save")
                }
            }
            .confirmationDialog("Delete this todo?", isPresented: $confirmDelete, titleVisibility: .visible) {
                Button("Delete", role: .destructive) {
                    store.todos.remove(item.id)
                    dismiss()
                }
            }
            .onAppear { focused = true }
        }
        .presentationDetents([.medium, .large])
    }

    private var count: Int { text.trimmingCharacters(in: .whitespacesAndNewlines).count }

    private var changed: Bool {
        text.trimmingCharacters(in: .whitespacesAndNewlines) != item.text || (canAuto && auto != item.auto)
    }

    private var canSave: Bool {
        guard changed, store.todos.canWrite else { return false }
        if case .success = TodoText.validate(text) { return true }
        return false
    }

    private func save() {
        let todos = store.todos
        if text.trimmingCharacters(in: .whitespacesAndNewlines) != item.text {
            guard todos.edit(item.id, text: text) == nil else { return }
        }
        if canAuto, auto != item.auto { todos.setAuto(item.id, auto) }
        dismiss()
    }
}
