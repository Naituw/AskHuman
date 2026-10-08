// Run: xcrun swiftc -target arm64-apple-macosx11.0 src-tauri/swift/{AttachmentPreview,PopupCanvas}.swift
// scripts/popup-find-native-regression.swift -o /tmp/askhuman-find-native && /tmp/askhuman-find-native
import AppKit
import PDFKit
import WebKit

private struct Reply { let generation: UInt64; let total: Int; let current: Int; let status: Int32 }
private var replies: [Reply] = []
private var keys: [UInt16] = []
private let findReply: FindCallback = { _, _, generation, total, current, status in
    replies.append(Reply(generation: generation, total: total, current: current, status: status))
}
private let keyReply: KeyCallback = { code, _, _ in keys.append(code) }

@main struct NativeFindRegression {
    static func check(_ value: @autoclosure () -> Bool, _ name: String) {
        guard value() else { fputs("FAIL: \(name)\n", stderr); exit(1) }
        print("PASS: \(name)")
    }
    private static func awaitReply(_ generation: UInt64, status: Int32 = 0) -> Reply {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if let reply = replies.last(where: { $0.generation == generation && $0.status == status }) { return reply }
            RunLoop.current.run(until: Date().addingTimeInterval(0.02))
        }
        fputs("Timed out waiting for generation \(generation), status \(status)\n", stderr); exit(1)
    }
    static func writePDF(_ url: URL, _ pages: [String]) {
        let consumer = CGDataConsumer(url: url as CFURL)!
        var box = CGRect(x: 0, y: 0, width: 600, height: 800)
        let context = CGContext(consumer: consumer, mediaBox: &box, nil)!
        for text in pages {
            context.beginPDFPage(nil)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
            (text as NSString).draw(at: CGPoint(x: 30, y: 730), withAttributes: [.font: NSFont.systemFont(ofSize: 16)])
            NSGraphicsContext.restoreGraphicsState()
            context.endPDFPage()
        }
        context.closePDF()
    }
    static func main() {
        let app = NSApplication.shared; app.setActivationPolicy(.accessory)
        let fixtureDirectory = ProcessInfo.processInfo.environment["ASKHUMAN_FIND_FIXTURES"]
        let directory = fixtureDirectory.map { URL(fileURLWithPath: $0) }
            ?? FileManager.default.temporaryDirectory.appendingPathComponent("askhuman-find-\(UUID().uuidString)")
        try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if fixtureDirectory == nil { try? FileManager.default.removeItem(at: directory) } }
        let textPDF = directory.appendingPathComponent("text.pdf"), scanPDF = directory.appendingPathComponent("scan.pdf")
        writePDF(textPDF, ["alpha Alpha alpha", "beta alpha", "gamma alpha"])
        writePDF(scanPDF, [""])
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 1266, height: 620), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let webview = WKWebView(frame: window.contentView!.bounds); window.contentView!.addSubview(webview)
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.close() }
        let pointer = Unmanaged.passUnretained(window).toOpaque()
        func show(_ url: URL) {
            let result = "request".withCString { request in url.path.withCString { path in
                updatePreviewView(pointer, request, path, 566, 48, 700, 572, false, keyReply)
            } }
            check(result, "native view accepts attachment")
        }
        func find(_ generation: UInt64, query: String, sensitive: Bool = false, action: String = "search", delta: Int = 0, navigate: Bool = true) {
            let result = "request".withCString { request in action.withCString { action in query.withCString { query in
                findInPreview(request, 0, generation, action, query, sensitive, navigate, delta, findReply)
            } } }
            check(result, "native command \(action) accepted")
        }
        show(textPDF); find(1, query: "alpha")
        check(awaitReply(1).total == 5, "case-insensitive search includes all pages")
        let pdf = window.contentView!.subviews.compactMap { $0 as? PDFView }.first!
        check(pdf.highlightedSelections?.count == 5, "native search highlights all five hits")
        find(1, query: "alpha", action: "go", delta: -1)
        check(replies.last?.current == 4, "previous wraps to the final hit")
        check(pdf.currentPage === pdf.document?.page(at: 2), "navigation selects a later PDF page")
        find(2, query: "alpha", sensitive: true)
        check(awaitReply(2).total == 4, "Aa excludes the differently cased hit")
        find(3, query: "alpha"); find(4, query: "beta")
        check(awaitReply(4).total == 1, "replacement query has its own result set")
        check(!replies.contains { $0.generation == 3 && $0.status == 0 }, "cancelled query never completes into its replacement")
        find(5, query: "missing")
        check(awaitReply(5).total == 0, "no match on a text PDF is a valid zero result")
        find(6, query: "alpha", delta: 3, navigate: false)
        check(awaitReply(6).current == 3, "passive restore preserves the current logical hit")
        find(6, query: "", action: "focus")
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0,
            windowNumber: window.windowNumber, context: nil, characters: "f", charactersIgnoringModifiers: "f", isARepeat: false, keyCode: 3)!
        app.sendEvent(event)
        check(keys.contains(3), "Cmd+F is forwarded from the native responder")
        check(window.firstResponder === webview, "Cmd+F yields native focus to the WebView")
        show(scanPDF); find(7, query: "alpha")
        check(awaitReply(7, status: 4).total == 0, "image-only PDF reports no searchable text")
        find(8, query: "", action: "cancel")
        check(pdf.highlightedSelections == nil, "closing search clears native highlights")
        print("Native PDF find regression passed")
    }
}
