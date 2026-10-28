import os
import SwiftUI
import UIKit
import WebKit

private let logger = Logger(subsystem: "com.naituw.humaninloop.ios", category: "web-markdown")

/// The `WKWebView` behind `WebMarkdownView`: loads the bundled page (`Resources/WebMarkdown`), hands it
/// the text, and follows what it reports. The page has no other way to reach the app than the five
/// messages below, and no way out to the network or to other pages.
struct WebPage: UIViewRepresentable {
    let text: String
    let typeSize: DynamicTypeSize
    /// Seconds the page may take to say anything before it is given up.
    let timeout: Double
    @Binding var height: CGFloat
    @Binding var phase: WebPhase
    /// A link was tapped on the page.
    let onLink: (URL) -> Void
    /// The text was laid out for the first time.
    let onSettled: () -> Void

    /// What the page may tell the app (see `render.js`); `log` only ever lands in the system log.
    fileprivate static let messages = ["ready", "height", "rendered", "copy", "failed", "log"]

    /// Passes the page's script errors and policy violations on to the log, from before any of its own
    /// scripts run (a page that cannot start leaves no other trace).
    fileprivate static let diagnostics = WKUserScript(
        source: """
        (function () {
          function log(text) { try { window.webkit.messageHandlers.log.postMessage(String(text).slice(0, 500)); } catch (e) {} }
          window.addEventListener('error', function (e) { log('script error: ' + e.message + ' (' + (e.filename || '').split('/').pop() + ':' + e.lineno + ')'); });
          document.addEventListener('securitypolicyviolation', function (e) { log('blocked by the page policy: ' + e.violatedDirective + ' ' + (e.blockedURI || '').slice(0, 80)); });
        })();
        """,
        injectionTime: .atDocumentStart,
        forMainFrameOnly: true
    )

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> WKWebView {
        let coordinator = context.coordinator
        let controller = WKUserContentController()
        let proxy = ScriptProxy(coordinator)
        for name in Self.messages { controller.add(proxy, name: name) }
        controller.addUserScript(Self.diagnostics)
        let configuration = WKWebViewConfiguration()
        configuration.userContentController = controller
        // Nothing the page does is kept: no cookies, no cache on disk.
        configuration.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.isOpaque = false
        view.backgroundColor = .clear
        let scroll = view.scrollView
        scroll.backgroundColor = .clear
        // The screen scrolls, not the page: its height is that of its text.
        scroll.isScrollEnabled = false
        scroll.bounces = false
        scroll.contentInsetAdjustmentBehavior = .never
        view.allowsLinkPreview = false
        view.navigationDelegate = coordinator
        #if DEBUG
        view.isInspectable = true
        #endif
        coordinator.attach(view, page: self)
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {
        context.coordinator.update(self)
    }

    static func dismantleUIView(_ view: WKWebView, coordinator: Coordinator) {
        coordinator.close(view)
    }

    // MARK: coordinator

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate {
        private weak var webView: WKWebView?
        private var page: WebPage?
        private var ready = false
        private var drawn: (text: String, typeSize: DynamicTypeSize)?
        /// How often the page was loaded again after its web content process ended.
        private var reloads = 0
        private var watchdog: Task<Void, Never>?

        /// Text a copy button may hand over, in UTF-16 units (a code block is far smaller than this).
        private static let copyLimit = 1_000_000

        func attach(_ view: WKWebView, page: WebPage) {
            webView = view
            self.page = page
            load()
        }

        func update(_ page: WebPage) {
            self.page = page
            renderIfNeeded()
        }

        func close(_ view: WKWebView) {
            watchdog?.cancel()
            view.stopLoading()
            view.navigationDelegate = nil
            view.configuration.userContentController.removeAllScriptMessageHandlers()
        }

        private func load() {
            guard let webView else { return }
            ready = false
            drawn = nil
            guard let directory = WebAssets.directory else {
                fail("the page is not part of the app")
                return
            }
            logger.notice("loading the page (load \(self.reloads + 1))")
            webView.loadFileURL(directory.appendingPathComponent("index.html"), allowingReadAccessTo: directory)
            // A page that says nothing is not waited for: the native renderer takes over.
            watchdog?.cancel()
            let timeout = page?.timeout ?? 5
            watchdog = Task { [weak self] in
                try? await Task.sleep(for: .seconds(timeout))
                guard !Task.isCancelled else { return }
                self?.fail("the page did not draw within \(timeout) seconds")
            }
        }

        private func renderIfNeeded() {
            guard ready, let page, let webView else { return }
            if let drawn, drawn.text == page.text, drawn.typeSize == page.typeSize { return }
            drawn = (page.text, page.typeSize)
            logger.notice("drawing \(page.text.count) characters")
            let script = "AH.render(\(WebStrings.literal(page.text)), {strings: \(WebStrings.words)})"
            // (Not the async variant: it traps when the script's result is `undefined`, as `render`'s is.)
            webView.evaluateJavaScript(script) { [weak self] _, error in
                if let error { self?.fail("the page could not be told the text: \(error.localizedDescription)") }
            }
        }

        /// Gives the page up: the native renderer draws the text from now on. (Not done inside a view
        /// update, which is where this may be called from.)
        private func fail(_ reason: String) {
            watchdog?.cancel()
            logger.notice("using the native renderer: \(reason, privacy: .public)")
            Task { [weak self] in self?.page?.phase = .failed }
        }

        fileprivate func handle(_ name: String, body: Any) {
            switch name {
            case "ready":
                logger.notice("the page is ready")
                ready = true
                renderIfNeeded()
            case "height":
                report(body, settled: false)
            case "rendered":
                report(body, settled: true)
            case "copy":
                copy(body)
            case "failed":
                fail("the page said so: \(body)")
            case "log":
                logger.notice("page: \(String(describing: body), privacy: .public)")
            default:
                break
            }
        }

        private func report(_ body: Any, settled: Bool) {
            watchdog?.cancel()
            guard let page, let value = (body as? NSNumber)?.doubleValue, value.isFinite else { return }
            let height = CGFloat(min(max(value, 0), Double(WebHeights.limit)))
            logger.notice("\(settled ? "laid out" : "height"): \(Int(height)) pt")
            if abs(page.height - height) > 0.5 { page.height = height }
            WebHeights.store(height, text: page.text)
            if page.phase == .loading { page.phase = .shown }
            if settled { page.onSettled() }
        }

        private func copy(_ body: Any) {
            guard let text = body as? String, text.utf16.count <= Self.copyLimit else { return }
            UIPasteboard.general.string = text
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        }

        // MARK: WKNavigationDelegate

        func webView(
            _ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction
        ) async -> WKNavigationActionPolicy {
            if navigationAction.navigationType == .linkActivated {
                if let url = navigationAction.request.url { page?.onLink(url) }
                return .cancel
            }
            // The only other thing the page may do is to be loaded (again, if its process was lost).
            guard let url = navigationAction.request.url, url.isFileURL, url.lastPathComponent == "index.html" else {
                return .cancel
            }
            return .allow
        }

        func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
            guard reloads < 2 else {
                fail("the web content process kept ending")
                return
            }
            reloads += 1
            load()
        }

        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            fail("the page did not load: \(error.localizedDescription)")
        }

        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
            fail("the page did not load: \(error.localizedDescription)")
        }
    }
}

/// Script messages keep their handler alive; this one lets go of the page's coordinator.
@MainActor
private final class ScriptProxy: NSObject, WKScriptMessageHandler {
    weak var target: WebPage.Coordinator?

    init(_ target: WebPage.Coordinator) { self.target = target }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        target?.handle(message.name, body: message.body)
    }
}

/// Where the page is in the app.
private enum WebAssets {
    /// The `WebMarkdown` folder of the app bundle (a folder reference, so that it keeps its shape).
    static var directory: URL? { Bundle.main.url(forResource: "WebMarkdown", withExtension: nil) }
}

/// What the app tells the page.
enum WebStrings {
    /// `value` as a JavaScript string literal.
    static func literal(_ value: String) -> String {
        guard let data = try? JSONEncoder().encode(value), let text = String(data: data, encoding: .utf8) else { return "\"\"" }
        return text
    }

    /// The words the page shows itself (the copy button's label, the diagram's buttons and notes), in
    /// the phone's language, as a JavaScript object literal.
    static var words: String {
        let words: [String: String] = [
            "copy": String(localized: "Copy"),
            "showSource": String(localized: "Show Mermaid source"),
            "showDiagram": String(localized: "Show diagram"),
            "drawing": String(localized: "Rendering diagram…"),
            "tooMany": String(localized: "Diagram limit reached; showing source"),
            "tooLarge": String(localized: "Diagram exceeds the safety limit; showing source"),
            "failed": String(localized: "Diagram could not be rendered; showing source"),
            "unavailable": String(localized: "Diagrams cannot be drawn here; showing source"),
        ]
        guard let data = try? JSONEncoder().encode(words), let text = String(data: data, encoding: .utf8) else { return "{}" }
        return text
    }
}
