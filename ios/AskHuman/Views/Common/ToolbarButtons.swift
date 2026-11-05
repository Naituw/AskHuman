import SwiftUI

// The buttons of a sheet's top bar (docs/specs/ios-large-screen.md §6).
//
// On the iPhone Duo's cover screen the system keeps a strip of buttons beside the page, within reach of the
// thumb. It moves a bar item there only when the item is an icon or has a role; a plain text button stays in
// the top bar, out of reach. So from iOS 26 on these are role buttons (✕ to back out, a blue ✓ to save, a blue
// ↑ to send); older systems have no such strip and keep the text buttons they always had.
//
// Put `disabled`, `accessibilityIdentifier` and the like on the call site, as with any button.

/// Backs out of a sheet without keeping anything: ✕ from iOS 26, "Cancel" before.
struct CancelToolbarButton: View {
    let action: () -> Void

    var body: some View {
        if #available(iOS 26, *) {
            Button(role: .cancel, action: action)
        } else {
            Button("Cancel", action: action)
        }
    }
}

/// Leaves a page that only shows something (a picture, how a task is doing): ✕ from iOS 26, "Close" before.
struct CloseToolbarButton: View {
    let action: () -> Void

    var body: some View {
        if #available(iOS 26, *) {
            Button(role: .close, action: action)
        } else {
            Button("Close", action: action)
        }
    }
}

/// Finishes a sheet: the system's prominent button from iOS 26 (a blue ✓ to save, a blue ↑ to send), the title
/// before. The label names the action for VoiceOver and the UI tests; a role button without one is read as
/// "Done" whatever it does, and a role with a bare title would stay in the top bar instead of the strip.
struct ConfirmToolbarButton: View {
    let title: LocalizedStringKey
    let systemImage: String
    /// Whether the text button of older systems is semibold, as the one thing a sheet is for.
    let emphasized: Bool
    let action: () -> Void

    init(_ title: LocalizedStringKey, systemImage: String, emphasized: Bool = false, action: @escaping () -> Void) {
        self.title = title
        self.systemImage = systemImage
        self.emphasized = emphasized
        self.action = action
    }

    var body: some View {
        if #available(iOS 26, *) {
            Button(role: .confirm, action: action) {
                Label(title, systemImage: systemImage)
            }
        } else {
            Button(title, action: action)
                .fontWeight(emphasized ? .semibold : nil)
        }
    }
}

/// Switches a list into reordering: the icon of an order from iOS 26, which becomes a ✓ while the list is
/// being reordered; "Edit" and "Done" before.
struct ReorderToolbarButton: View {
    @Environment(\.editMode) private var editMode

    var body: some View {
        if #available(iOS 26, *) {
            let reordering = editMode?.wrappedValue.isEditing == true
            Button(role: reordering ? .confirm : nil) {
                withAnimation { editMode?.wrappedValue = reordering ? .inactive : .active }
            } label: {
                if reordering {
                    Label("Done", systemImage: "checkmark")
                } else {
                    Label("Reorder", systemImage: "arrow.up.arrow.down")
                }
            }
        } else {
            EditButton()
        }
    }
}
