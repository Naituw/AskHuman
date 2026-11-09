import UIKit

/// What a request page shares with UIKit: the editor that has the keyboard, and whatever is put over the page.
@MainActor
enum PageEditing {
    private static var window: UIWindow? {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows).first(where: \.isKeyWindow)
    }

    /// Whether something is over the page or the reader is typing on it: a sheet, an alert, a full-screen cover, the
    /// keyboard. The page must not be moved from under that: what is being written would go with it
    /// (docs/specs/ios-answer-flow.md §3.8).
    static var isCovered: Bool {
        guard let window else { return false }
        if let presented = window.rootViewController?.presentedViewController, !presented.isBeingDismissed { return true }
        return firstResponder(in: window) is UITextInput
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
