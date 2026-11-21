import UIKit

/// The system's share sheet for a file, opened from code: sharing a picture of which the phone holds a preview first has to
/// fetch the file, and a `ShareLink` cannot be opened once that is done (docs/specs/ios-attachment-traffic.md §7.2).
@MainActor
enum SharePanel {
    /// Opens the share sheet over whatever is on top, the viewer's sheet. On an iPad it hangs from the top right corner of
    /// that, just below where the viewer's share button is. Does nothing while a share sheet is open already.
    static func present(_ url: URL) {
        guard let window = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first(where: { $0.activationState == .foregroundActive })?
            .windows.first(where: \.isKeyWindow),
              var top = window.rootViewController
        else { return }
        while let presented = top.presentedViewController, !presented.isBeingDismissed { top = presented }
        guard !(top is UIActivityViewController) else { return }

        let sheet = UIActivityViewController(activityItems: [url], applicationActivities: nil)
        if let popover = sheet.popoverPresentationController {
            let bounds = top.view.bounds
            popover.sourceView = top.view
            popover.sourceRect = CGRect(x: bounds.maxX - 36, y: top.view.safeAreaInsets.top + 48, width: 1, height: 1)
            popover.permittedArrowDirections = .up
        }
        top.present(sheet, animated: true)
    }
}
