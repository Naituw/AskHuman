import SwiftUI
import UIKit
import AHCore
import AHProtocol

/// One todo of the list on a request page: an option to pick (the whole row is the target), and the same three actions as
/// on the Todos tab (docs/specs/todo-list-actions.md D12): swipe right to complete (all the way across does it), swipe left
/// for "Delete", or press and hold for a menu. VoiceOver gets them as custom actions.
struct TodoActionRow: View {
    let picker: RequestTodoPicker
    let todo: TodoItem
    /// The row's identity on screen, which a todo the phone just added keeps when the Mac gives it its own id.
    let key: String
    /// Its place in the list, for the UI tests.
    let index: Int
    let single: Bool
    /// The form is disabled while an answer is on its way, and once the request has ended.
    @Environment(\.isEnabled) private var formEnabled

    var body: some View {
        let acts = picker.canAct(on: todo.id) && formEnabled
        SwipeActionRow(
            id: key,
            isEnabled: acts,
            leading: SwipeRowAction(
                title: "Complete", symbol: "checkmark", tint: .green, identifier: "todo-swipe-complete-\(index)", fullSwipe: true
            ) { picker.complete(todo.id) },
            trailing: SwipeRowAction(
                title: "Delete", symbol: "trash", tint: .red, identifier: "todo-swipe-delete-\(index)"
            ) { picker.remove(todo.id) }
        ) {
            OptionRow(
                todo: todo, single: single, selected: picker.isPicked(todo.id), sending: picker.isSending(todo.id)
            ) {
                picker.toggle(todo.id)
            }
            .accessibilityIdentifier("todo-row-\(index)")
            .accessibilityActions {
                if acts {
                    Button("Complete") { picker.complete(todo.id) }
                    Button("Delete") { picker.remove(todo.id) }
                }
                Button("Copy") { UIPasteboard.general.string = todo.text }
            }
            .overlay { lit }
        }
        .id("todo-\(key)")
        .contextMenu {
            Button {
                picker.complete(todo.id)
            } label: {
                Label("Complete", systemImage: "checkmark.circle")
            }
            .disabled(!acts)
            Button {
                UIPasteboard.general.string = todo.text
            } label: {
                Label("Copy", systemImage: "doc.on.doc")
            }
            Button(role: .destructive) {
                picker.remove(todo.id)
            } label: {
                Label("Delete", systemImage: "trash")
            }
            .disabled(!acts)
        }
    }

    /// The wash over a todo that was just added: it shows where the new row is, and fades.
    private var lit: some View {
        let lit = picker.isLit(todo.id)
        return RoundedRectangle(cornerRadius: 12, style: .continuous)
            .fill(Color.accentColor.opacity(0.22))
            .opacity(lit ? 1 : 0)
            .animation(.easeOut(duration: 0.6), value: lit)
            .allowsHitTesting(false)
    }
}
