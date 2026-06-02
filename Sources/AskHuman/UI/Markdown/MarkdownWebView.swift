import SwiftUI
import WebKit

/// 基于 WKWebView 的 Markdown 渲染（支持整段跨行选中）。WebView 高度自适应内容。
struct MarkdownWebContentView: View {
    let markdown: String
    let theme: ThemeMode

    @State private var height: CGFloat = 40

    var body: some View {
        WebViewRepresentable(markdown: markdown, theme: theme) { newHeight in
            if abs(newHeight - height) > 0.5 {
                height = newHeight
            }
        }
        .frame(height: height)
        .frame(maxWidth: .infinity)
    }
}

private struct WebViewRepresentable: NSViewRepresentable {
    let markdown: String
    let theme: ThemeMode
    let onHeightChange: (CGFloat) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onHeightChange: onHeightChange)
    }

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.userContentController.add(context.coordinator, name: "heightChanged")

        let webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = context.coordinator
        webView.setValue(false, forKey: "drawsBackground")
        webView.enclosingScrollView?.hasVerticalScroller = false
        applyAppearance(to: webView)
        webView.loadHTMLString(htmlDocument(), baseURL: nil)
        context.coordinator.lastHTML = htmlDocument()
        return webView
    }

    func updateNSView(_ webView: WKWebView, context: Context) {
        applyAppearance(to: webView)
        let html = htmlDocument()
        if html != context.coordinator.lastHTML {
            context.coordinator.lastHTML = html
            webView.loadHTMLString(html, baseURL: nil)
        }
    }

    private func applyAppearance(to webView: WKWebView) {
        switch theme {
        case .system: webView.appearance = nil
        case .light: webView.appearance = NSAppearance(named: .aqua)
        case .dark: webView.appearance = NSAppearance(named: .darkAqua)
        }
    }

    private func resolvedIsDark() -> Bool {
        switch theme {
        case .light: return false
        case .dark: return true
        case .system:
            let match = NSApp.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua])
            return match == .darkAqua
        }
    }

    private func htmlDocument() -> String {
        let body = HTMLMarkdownRenderer.html(from: markdown)
        let isDark = resolvedIsDark()
        let bodyClass = isDark ? "dark" : "light"
        let colorScheme = isDark ? "dark" : "light"
        return """
        <!DOCTYPE html>
        <html>
        <head>
        <meta charset="utf-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <style>
          :root { color-scheme: \(colorScheme); }
          html, body { margin: 0; padding: 0; background: transparent; }
          body {
            font-family: -apple-system, system-ui, sans-serif;
            font-size: 14px;
            line-height: 1.55;
            word-wrap: break-word;
            padding: 2px;
          }
          h1,h2,h3,h4,h5,h6 { margin: 0.6em 0 0.3em; line-height: 1.25; }
          h1 { font-size: 1.7em; } h2 { font-size: 1.45em; } h3 { font-size: 1.25em; }
          p { margin: 0.45em 0; }
          ul, ol { margin: 0.4em 0; padding-left: 1.4em; }
          li { margin: 0.15em 0; }
          code {
            font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
            font-size: 0.92em;
            padding: 0.1em 0.3em;
            border-radius: 4px;
          }
          pre { padding: 10px; border-radius: 6px; overflow-x: auto; }
          pre code { background: none; padding: 0; }
          blockquote { margin: 0.5em 0; padding-left: 0.8em; border-left: 3px solid; }
          table { border-collapse: collapse; margin: 0.5em 0; width: 100%; }
          th, td { border: 1px solid; padding: 5px 8px; text-align: left; }
          hr { border: none; border-top: 1px solid; margin: 0.8em 0; }
          img { max-width: 100%; }

          /* 浅色 */
          body.light { color: #1d1d1f; }
          body.light a { color: #0a84ff; }
          body.light code, body.light pre { background: rgba(0,0,0,0.06); }
          body.light blockquote { color: #555; border-left-color: rgba(0,0,0,0.2); }
          body.light th, body.light td { border-color: rgba(0,0,0,0.18); }
          body.light thead th { background: rgba(0,0,0,0.05); }
          body.light hr { border-top-color: rgba(0,0,0,0.15); }

          /* 深色 */
          body.dark { color: #e8e8ea; }
          body.dark a { color: #4ea1ff; }
          body.dark code, body.dark pre { background: rgba(255,255,255,0.12); }
          body.dark blockquote { color: #c2c2c6; border-left-color: rgba(255,255,255,0.3); }
          body.dark th, body.dark td { border-color: rgba(255,255,255,0.2); }
          body.dark thead th { background: rgba(255,255,255,0.08); }
          body.dark hr { border-top-color: rgba(255,255,255,0.2); }

          /* 代码块内的 code 不重复背景 */
          body.light pre code, body.dark pre code { background: transparent; padding: 0; border-radius: 0; }
        </style>
        </head>
        <body class="\(bodyClass)">
        \(body)
        <script>
          function reportHeight() {
            var h = document.body.scrollHeight;
            window.webkit.messageHandlers.heightChanged.postMessage(h);
          }
          window.addEventListener('load', reportHeight);
          if (window.ResizeObserver) {
            new ResizeObserver(reportHeight).observe(document.body);
          }
          setTimeout(reportHeight, 50);
        </script>
        </body>
        </html>
        """
    }

    final class Coordinator: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
        let onHeightChange: (CGFloat) -> Void
        var lastHTML: String = ""

        init(onHeightChange: @escaping (CGFloat) -> Void) {
            self.onHeightChange = onHeightChange
        }

        func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
            if message.name == "heightChanged", let value = message.body as? CGFloat {
                onHeightChange(value)
            } else if message.name == "heightChanged", let num = message.body as? NSNumber {
                onHeightChange(CGFloat(num.doubleValue))
            }
        }

        // 外部链接用系统浏览器打开
        func webView(_ webView: WKWebView,
                     decidePolicyFor navigationAction: WKNavigationAction,
                     decisionHandler: @escaping @MainActor @Sendable (WKNavigationActionPolicy) -> Void) {
            if navigationAction.navigationType == .linkActivated, let url = navigationAction.request.url {
                NSWorkspace.shared.open(url)
                decisionHandler(.cancel)
                return
            }
            decisionHandler(.allow)
        }
    }
}
