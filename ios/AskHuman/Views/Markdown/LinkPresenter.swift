import SwiftUI
import SafariServices

/// Links in Markdown: shows the destination's domain first, then opens it in an in-app browser.
@MainActor
@Observable
final class LinkPresenter {
    var pending: URL?
    var presented: URL?

    /// Called by the Markdown views; web links are held back for confirmation, others go to the system.
    func request(_ url: URL) -> OpenURLAction.Result {
        guard let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https" else { return .systemAction }
        pending = url
        return .handled
    }

    /// A link tapped on a web-drawn body: web links wait for confirmation like the others, mail and phone
    /// links go to the system, and anything else (a scheme only an app understands, or one that should
    /// not be followed from a page) is ignored.
    func tapped(_ url: URL) {
        switch url.scheme?.lowercased() {
        case "http", "https": pending = url
        case "mailto", "tel": UIApplication.shared.open(url)
        default: break
        }
    }

    var pendingDomain: String { pending?.host() ?? pending?.absoluteString ?? "" }
}

struct SafariView: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> SFSafariViewController {
        SFSafariViewController(url: url)
    }

    func updateUIViewController(_ controller: SFSafariViewController, context: Context) {}
}

extension URL: @retroactive Identifiable {
    public var id: String { absoluteString }
}
