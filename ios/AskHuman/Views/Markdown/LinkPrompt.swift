import SafariServices
import UIKit

/// What a followed link puts on the screen: the question before it leaves the message, and the in-app browser.
///
/// Both are presented from whatever is on top, not from the root of the app: a sheet that shows a Markdown
/// attachment has links of its own, and a presentation from the screen underneath it has nowhere to appear.
@MainActor
enum LinkPrompt {
    /// Asks before a link is followed. On a narrow screen this is the system's action sheet; on a wide one it is a
    /// popover whose arrow points at `anchor`, the spot (in window coordinates) where the link was tapped.
    static func ask(title: String, message: String, open: String, near anchor: CGPoint?, confirmed: @escaping () -> Void) {
        guard let host = top(), !(host is UIAlertController) else { return }
        let sheet = UIAlertController(title: title, message: message, preferredStyle: .actionSheet)
        sheet.view.accessibilityIdentifier = "link-prompt"
        sheet.addAction(UIAlertAction(title: open, style: .default) { _ in confirmed() })
        sheet.addAction(UIAlertAction(title: String(localized: "Cancel"), style: .cancel))
        // (A popover with no source is an error on a wide screen; an action sheet ignores it.)
        if let popover = sheet.popoverPresentationController {
            popover.sourceView = host.view
            if let anchor {
                popover.sourceRect = spot(at: anchor, in: host.view)
            } else {
                // With nothing to point at, it sits in the middle of the screen and has no arrow: an arrow at a
                // place that is not the link would be worse than none.
                popover.sourceRect = CGRect(x: host.view.bounds.midX, y: host.view.bounds.midY, width: 0, height: 0)
                popover.permittedArrowDirections = []
            }
        }
        host.present(sheet, animated: true)
    }

    /// Shows a web address in the in-app browser. Only http(s) addresses may be given: the browser refuses the rest.
    static func showBrowser(_ url: URL) {
        guard let host = top() else { return }
        let browser = SFSafariViewController(url: url)
        browser.modalPresentationStyle = .pageSheet
        host.present(browser, animated: true)
    }

    /// The controller that is in front of the person: the last of the chain of presented ones.
    private static func top() -> UIViewController? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let window = scenes.flatMap(\.windows).first(where: \.isKeyWindow) ?? scenes.first?.windows.first
        var top = window?.rootViewController
        while let next = top?.presentedViewController, !next.isBeingDismissed { top = next }
        return top
    }

    /// What the popover points at: a line's height around the touch, so that its arrow stops short of the text that
    /// was tapped instead of covering it.
    private static func spot(at point: CGPoint, in view: UIView) -> CGRect {
        view.convert(CGRect(x: point.x - 1, y: point.y - 10, width: 2, height: 20), from: nil)
    }
}
