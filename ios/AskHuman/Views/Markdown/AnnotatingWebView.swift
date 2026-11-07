import UIKit
import WebKit
import AHCore
import AHProtocol

/// The web view of a message that takes notes: the menu of a text selection (copy, look up, …) gets one more entry,
/// "Add annotation", or "Edit annotation" when the passage is one that has a note already
/// (docs/specs/message-annotations.md D17).
///
/// The system builds the menu when the selection settles. What the page said about the selection has arrived by
/// then (it is told within a frame of the selection changing), so the entry is decided from that: by `offer`, which
/// the page's coordinator answers from the form.
final class AnnotatingWebView: WKWebView {
    /// What the menu offers for the selection as it is now; `nil` for nothing (no selection, no notes on this request).
    var offer: (() -> NotesPage.Offer?)?
    /// The person chose the entry.
    var choose: ((NotesPage.Offer) -> Void)?

    override func buildMenu(with builder: any UIMenuBuilder) {
        super.buildMenu(with: builder)
        // The menu of a selection, not the menu bar.
        guard builder.system == UIMenuSystem.context, let offer = offer?() else { return }
        let entry: UIAction
        switch offer {
        case .add:
            entry = UIAction(title: String(localized: "Add annotation"), image: UIImage(systemName: "text.bubble")) { [weak self] _ in
                self?.choose?(offer)
            }
        case .edit:
            entry = UIAction(title: String(localized: "Edit annotation"), image: UIImage(systemName: "text.bubble")) { [weak self] _ in
                self?.choose?(offer)
            }
        case .full:
            // Said, not left out: a missing entry would look like a fault.
            entry = UIAction(
                title: String(localized: "Annotation limit reached (\(Wire.maxAnnotations))"),
                image: UIImage(systemName: "text.bubble"), attributes: .disabled
            ) { _ in }
        }
        builder.insertChild(UIMenu(title: "", options: .displayInline, children: [entry]), atStartOfMenu: .standardEdit)
    }
}
