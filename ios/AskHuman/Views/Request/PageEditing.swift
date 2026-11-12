import UIKit

/// What a request page shares with UIKit: the editor that has the keyboard, and whatever is put over the page.
@MainActor
enum PageEditing {
    private static var window: UIWindow? {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows).first(where: \.isKeyWindow)
    }

    static var isVisible: Bool {
        UIApplication.shared.applicationState == .active && window?.windowScene?.activationState == .foregroundActive
    }

    /// Whether something is over the page or the reader is typing on it: a sheet, an alert, a full-screen cover, the
    /// keyboard. The page must not be moved from under that: what is being written would go with it
    /// (docs/specs/ios-answer-flow.md §3.8).
    static var isCovered: Bool {
        guard let window else { return false }
        if let presented = window.rootViewController?.presentedViewController, !presented.isBeingDismissed { return true }
        // Read-only WKContentView also adopts UITextInput and becomes first responder on a tap, without a keyboard.
        // Protect the app's actual editors, including when a hardware keyboard is used, rather than any text responder.
        switch firstResponder(in: window) {
        case let editor as UITextView: return editor.isEditable
        case let field as UITextField: return field.isEnabled
        default: return false
        }
    }

    /// Lets the editor under the keyboard publish its final composition, and puts the keyboard away. Before the
    /// page is replaced by another.
    static func commit() {
        guard let window else { return }
        (firstResponder(in: window) as? UITextInput)?.unmarkText()
        window.endEditing(true)
    }

    private static func firstResponder(in view: UIView) -> UIView? {
        if view.isFirstResponder { return view }
        for child in view.subviews { if let found = firstResponder(in: child) { return found } }
        return nil
    }
}
