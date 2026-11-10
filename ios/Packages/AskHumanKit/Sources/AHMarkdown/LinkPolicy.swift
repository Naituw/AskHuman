import Foundation

/// What the app does with a link tapped in a message.
public enum LinkAction: Equatable, Sendable {
    /// A web address: its domain is shown first, then it opens in the in-app browser.
    case browser
    /// Mail and phone links: the system asks for itself (a draft to send, a call to place), so nothing is added.
    case system
    /// Any other scheme: the whole address is shown first, then the system hands it to the app that claims the
    /// scheme. There is no list of schemes to allow: opening needs none (only asking beforehand whether an app
    /// would, which is not done), and the person sees where a link goes before it leaves the app.
    case app
    /// Nothing to follow: no scheme, or one that runs script or reads local data instead of leading to an app.
    case ignore
}

/// One answer for both renderers of Markdown (the web page and the native text), so that a link does the same
/// wherever it is drawn (docs/specs/ios-web-markdown.md W7).
public enum LinkPolicy {
    /// The schemes the web page's Markdown engine refuses to make links of (markdown-it's `validateLink`), and which
    /// are not worth a confirmation either: they open nothing. This is not a list of schemes that may be followed.
    static let refused: Set<String> = ["javascript", "vbscript", "file", "data"]

    public static func action(for url: URL) -> LinkAction {
        guard let scheme = url.scheme?.lowercased(), !scheme.isEmpty else { return .ignore }
        switch scheme {
        case "http", "https": return .browser
        case "mailto", "tel": return .system
        default: return refused.contains(scheme) ? .ignore : .app
        }
    }
}
