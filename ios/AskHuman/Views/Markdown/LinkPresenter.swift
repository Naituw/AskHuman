import SwiftUI
import AHMarkdown

/// Links in messages: what happens when one is tapped, whichever renderer drew it. `LinkPolicy` decides, so that the
/// web page and the native text treat a link alike (docs/specs/ios-web-markdown.md W7).
@MainActor
@Observable
final class LinkPresenter {
    /// Called when a link was handed to the system and no app took it. (Set by `RootView`, which can tell the person.)
    @ObservationIgnored var onUnopened: ((URL) -> Void)?

    /// Called by the native renderer: SwiftUI asks what to do with a link of its text.
    func request(_ url: URL) -> OpenURLAction.Result {
        switch LinkPolicy.action(for: url) {
        case .system:
            return .systemAction
        case .ignore:
            return .handled
        case .browser, .app:
            ask(url)
            return .handled
        }
    }

    /// Called by the web page's coordinator: a link was tapped on a body it drew.
    func tapped(_ url: URL) {
        switch LinkPolicy.action(for: url) {
        case .system: UIApplication.shared.open(url)
        case .browser, .app: ask(url)
        case .ignore: break
        }
    }

    /// Shows what the link is, and where it goes, before it is followed.
    private func ask(_ url: URL) {
        let anchor = LastTouch.shared.recent()
        if LinkPolicy.action(for: url) == .browser {
            let domain = url.host() ?? url.absoluteString
            LinkPrompt.ask(
                title: String(localized: "Open this link?"), message: url.absoluteString,
                open: String(localized: "Open \(domain)"), near: anchor
            ) { LinkPrompt.showBrowser(url) }
        } else {
            LinkPrompt.ask(
                title: String(localized: "Open in another app?"), message: url.absoluteString,
                open: String(localized: "Open"), near: anchor
            ) { [weak self] in self?.open(url) }
        }
    }

    /// Hands the address to the system, which gives it to the app that has claimed its scheme. (`canOpenURL` is not
    /// asked first: it only answers for schemes the app has declared in advance, and a link can be to any.)
    private func open(_ url: URL) {
        UIApplication.shared.open(url, options: [:]) { [weak self] opened in
            if !opened { self?.onUnopened?(url) }
        }
    }
}
