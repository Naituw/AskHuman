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
