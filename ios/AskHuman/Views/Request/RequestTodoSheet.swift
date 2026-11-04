import SwiftUI
import AHCore

/// The "Add todo…" sheet of a request page (docs/specs/todo-quick-add.md §4.2): the project the request belongs
/// to, and one sentence. It only draws what `RequestTodoComposer` says; the writing is the composer's. There is
/// nothing to choose: the todo goes to the request's project, on the Mac the request came from.
struct RequestTodoSheet: View {
    let composer: RequestTodoComposer
    /// The Mac took the todo (the project's name): the page closes the sheet and says so.
    let added: (String) -> Void
    let cancel: () -> Void
    @FocusState private var focused: Bool

    var body: some View {
        @Bindable var composer = composer
        let sending = composer.phase == .sending
        NavigationStack {
            Form {
                Section {
                    RequestTodoProjectRow(project: composer.projectName, mac: composer.macName)
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("request-todo-project")
                }
                Section {
                    TextField("New todo", text: $composer.text, axis: .vertical)
                        .lineLimit(3...10)
                        .focused($focused)
                        .disabled(sending)
                        .accessibilityIdentifier("request-todo-field")
                } footer: {
                    RequestTodoNote(composer: composer)
                }
            }
            .navigationTitle("New todo")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: cancel)
                        .disabled(sending)
                        .accessibilityIdentifier("request-todo-cancel")
                }
                ToolbarItem(placement: .confirmationAction) {
                    if sending {
                        ProgressView().accessibilityIdentifier("request-todo-sending")
                    } else {
                        Button("Add") { Task { if await composer.send() { added(composer.projectName) } } }
                            .disabled(!composer.canSend)
                            .accessibilityIdentifier("request-todo-send")
                    }
                }
            }
        }
        .presentationDetents([.medium, .large])
        // While the Mac is being asked there is no way out: a refusal that came after the sheet was gone
        // would have nowhere to say so.
        .interactiveDismissDisabled(sending)
        .task { focused = true }
    }
}

/// The line that says where the todo goes: fixed, so there is nothing to tap.
private struct RequestTodoProjectRow: View {
    let project: String
    let mac: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "folder")
                .font(.title3)
                .foregroundStyle(.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: project).font(.body.weight(.medium)).lineLimit(1)
                Text(verbatim: mac).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 8)
        }
    }
}

/// Why the todo was not sent, or cannot be yet. Nothing for an empty text: it needs no explaining.
private struct RequestTodoNote: View {
    let composer: RequestTodoComposer

    var body: some View {
        if let failure = composer.failure {
            Text(failure.text)
                .foregroundStyle(.red)
                .accessibilityIdentifier("request-todo-note")
        } else if let reason = composer.unavailable {
            Text(reason.text)
                .accessibilityIdentifier("request-todo-note")
        }
    }
}

/// "Added to AskHuman": what floats down over the top of a request page when the Mac took the todo.
struct TodoAddedPill: View {
    let project: String

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
            Text("Added to \(project)")
        }
        .font(.subheadline.weight(.medium))
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(.regularMaterial, in: Capsule())
        .shadow(color: .black.opacity(0.12), radius: 8, y: 2)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("todo-added-pill")
    }
}
