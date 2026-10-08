import AppKit
import ImageIO
import QuickLookUI
import PDFKit
import QuartzCore

// The result buffer belongs to Swift and is valid only during this synchronous callback.
typealias ImageCallback = @convention(c) (UnsafePointer<UInt8>?, Int, UInt32, UInt32, Int32) -> Void

@_cdecl("ah_preview_decode_image")
func decodePreviewImage(_ bytes: UnsafePointer<UInt8>, _ length: Int, _ thumbnail: Bool,
                        _ callback: ImageCallback) {
    autoreleasepool {
        let data = Data(bytes: bytes, count: length)
        guard let source = CGImageSourceCreateWithData(data as CFData,
                [kCGImageSourceShouldCache: false] as CFDictionary),
              let type = CGImageSourceGetType(source) as String? else {
            // A system preview extension may support formats absent from Image I/O.
            callback(nil, 0, 0, 0, 4); return
        }
        let count = CGImageSourceGetCount(source)
        let icon = type == "com.apple.icns" || type == "com.microsoft.ico" || type == "com.microsoft.cur"
        // Preserve multipage and sequence semantics in Quick Look instead of flattening them.
        guard count > 0 else { callback(nil, 0, 0, 0, 1); return }
        guard count <= 500 else { callback(nil, 0, 0, 0, 3); return }
        var total: UInt64 = 0
        var selected = 0
        var largest: UInt64 = 0
        var maxDimension = 0
        for i in 0..<count {
            guard let properties = CGImageSourceCopyPropertiesAtIndex(source, i, nil) as? [CFString: Any],
                  let width = properties[kCGImagePropertyPixelWidth] as? NSNumber,
                  let height = properties[kCGImagePropertyPixelHeight] as? NSNumber else { continue }
            let w = width.uint64Value, h = height.uint64Value
            guard w > 0, h > 0, w <= 40_000_000, h <= 40_000_000,
                  w <= 40_000_000 / h else { callback(nil, 0, 0, 0, 3); return }
            total += w * h
            if total > 80_000_000 { callback(nil, 0, 0, 0, 3); return }
            if w * h > largest { largest = w * h; selected = i; maxDimension = max(width.intValue, height.intValue) }
        }
        if count > 1 && !icon { callback(nil, 0, UInt32(count), 0, 2); return }
        guard largest > 0 else { callback(nil, 0, 0, 0, 1); return }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: thumbnail ? min(maxDimension, 192) : maxDimension,
            kCGImageSourceShouldCacheImmediately: true
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, selected, options as CFDictionary) else {
            callback(nil, 0, 0, 0, 1); return
        }
        let png = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(png, "public.png" as CFString, 1, nil) else {
            callback(nil, 0, 0, 0, 1); return
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { callback(nil, 0, 0, 0, 1); return }
        guard png.length <= 20 * 1024 * 1024 else { callback(nil, 0, 0, 0, 3); return }
        callback(png.bytes.assumingMemoryBound(to: UInt8.self), png.length,
                 UInt32(image.width), UInt32(image.height), 0)
    }
}

typealias KeyCallback = @convention(c) (UInt16, UInt64, UnsafePointer<CChar>?) -> Void
typealias FindCallback = @convention(c) (UnsafePointer<CChar>, Int, UInt64, Int, Int, Int32) -> Void
private var nativePreview: EmbeddedPreview?
private final class PDFLoadTicket {
    private let lock = NSLock()
    private var cancelled = false
    func cancel() { lock.lock(); cancelled = true; lock.unlock() }
    var isCancelled: Bool { lock.lock(); defer { lock.unlock() }; return cancelled }
}

// Each query owns a distinct PDFDocument. A cancelled query's late delegate notifications
// cannot be mistaken for notifications from its replacement on the displayed document.
private final class PDFSearchTask: NSObject, PDFDocumentDelegate {
    weak var owner: EmbeddedPreview?
    let request: String
    let index: Int
    let generation: UInt64
    let callback: FindCallback
    let ticket = PDFLoadTicket()
    var document: PDFDocument?
    var hasText = false
    var navigate: Bool
    let initial: Int
    init(owner: EmbeddedPreview, index: Int, generation: UInt64, navigate: Bool, initial: Int, callback: @escaping FindCallback) {
        self.owner = owner; self.request = owner.requestID; self.index = index
        self.generation = generation; self.navigate = navigate; self.initial = initial; self.callback = callback
    }
    func emit(_ status: Int32) {
        guard !ticket.isCancelled, let owner, owner.searchTask === self else { return }
        request.withCString { callback($0, index, generation, owner.findResults.count, owner.findCurrent, status) }
    }
    func didMatchString(_ selection: PDFSelection) {
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.ticket.isCancelled, let owner = self.owner,
                  owner.searchTask === self, let document = self.document,
                  let live = owner.pdf.document else { return }
            if owner.findResults.count >= 50_000 {
                self.emit(6); self.cancel(); owner.findResults.removeAll(); return
            }
            let mapped = PDFSelection(document: live)
            for page in selection.pages {
                guard let target = live.page(at: document.index(for: page)) else { continue }
                for i in 0..<selection.numberOfTextRanges(on: page) {
                    if let part = target.selection(for: selection.range(at: i, on: page)) { mapped.add(part) }
                }
            }
            if !mapped.pages.isEmpty { owner.findResults.append(mapped) }
        }
    }
    func documentDidEndDocumentFind(_ notification: Notification) {
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.ticket.isCancelled, let owner = self.owner, owner.searchTask === self else { return }
            owner.findCurrent = owner.findResults.isEmpty ? -1 : min(max(0, self.initial), owner.findResults.count - 1)
            owner.paintFind(navigate: self.navigate)
            self.emit(self.hasText ? 0 : 4)
            self.document?.delegate = nil; self.document = nil
        }
    }
    func cancel() { ticket.cancel(); document?.cancelFindString(); document?.delegate = nil; document = nil }
}

private final class EmbeddedPreview {
    let requestID: String
    var view: QLPreviewView
    let defaultView: QLPreviewView
    var cachedViews: [(String, QLPreviewView)] = []
    let pdf = PDFView(frame: .zero)
    let spinner = NSProgressIndicator(frame: .zero)
    let pdfQueue = DispatchQueue(label: "com.askhuman.preview.pdf", qos: .userInitiated)
    var pdfLoad: PDFLoadTicket?
    var pdfActive = false
    var pdfSource: Data?
    var surface: NSView { pdfActive ? pdf : view }
    struct PDFState { let page: Int; let point: CGPoint; let scale: CGFloat; let automatic: Bool }
    var pdfStates: [String: PDFState] = [:]
    weak var window: NSWindow?
    var path: String?
    var states: [String: Any] = [:]
    var monitor: Any?
    var interactionMonitor: Any?
    var searchTask: PDFSearchTask?
    var findResults: [PDFSelection] = []
    var findCurrent = -1
    var findGeneration: UInt64 = 0
    var pendingFind: (() -> Void)?
    var submitWithBareEnter = false
    let keyCallback: KeyCallback

    init(window: NSWindow, requestID: String, keyCallback: @escaping KeyCallback) {
        self.window = window
        self.requestID = requestID
        self.keyCallback = keyCallback
        defaultView = QLPreviewView(frame: .zero, style: .normal)!
        view = defaultView
        view.autostarts = false
        view.shouldCloseWithWindow = true
        popupPreviewParent(window)?.addSubview(view)
        pdf.displayMode = .singlePageContinuous
        pdf.autoScales = true
        pdf.isHidden = true
        pdf.backgroundColor = .windowBackgroundColor
        popupPreviewParent(window)?.addSubview(pdf)
        spinner.style = .spinning
        spinner.isHidden = true
        popupPreviewParent(window)?.addSubview(spinner)
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, !self.surface.isHidden, event.window === self.window,
                  let responder = self.window?.firstResponder as? NSView,
                  responder === self.surface || responder.isDescendant(of: self.surface) else { return event }
            if let input = responder as? NSTextInputClient, input.hasMarkedText() { return event }
            let mods = event.modifierFlags.intersection([.command, .control, .option, .shift])
            let enter = event.keyCode == 36 || event.keyCode == 76
            let submit = enter && (self.submitWithBareEnter ? mods.isEmpty : mods == .command)
            let find = (event.keyCode == 3 && mods == .command)
                || (event.keyCode == 5 && (mods == .command || mods == [.command, .shift]))
            if (event.keyCode == 53 && mods.isEmpty) || submit || (event.keyCode == 13 && mods == .command) || find {
                let selection = self.pdfActive ? String((self.pdf.currentSelection?.string ?? "").prefix(200)) : ""
                if event.keyCode == 3 { self.focusWebView() }
                selection.withCString { self.keyCallback(event.keyCode, UInt64(event.modifierFlags.rawValue), $0) }
                return nil
            }
            return event
        }
        interactionMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .scrollWheel]) { [weak self] event in
            guard let self, !self.surface.isHidden, event.window === self.window else { return event }
            let point = self.surface.convert(event.locationInWindow, from: nil)
            if self.surface.bounds.contains(point) { self.keyCallback(0, 0, nil) }
            return event
        }
    }
    func focusWebView() {
        if let webview = window?.contentView.flatMap({ webView(in: $0) }) { window?.makeFirstResponder(webview) }
    }
    func cancelSearch() {
        pendingFind = nil
        searchTask?.cancel(); searchTask = nil
        if !findResults.isEmpty { pdf.clearSelection() }
        findResults.removeAll(); findCurrent = -1; pdf.highlightedSelections = nil
    }
    func paintFind(navigate: Bool) {
        for (i, selection) in findResults.enumerated() {
            selection.color = (i == findCurrent ? NSColor.systemOrange : NSColor.systemYellow).withAlphaComponent(0.45)
        }
        pdf.highlightedSelections = findResults
        if findCurrent >= 0 && findCurrent < findResults.count && navigate {
            pdf.setCurrentSelection(findResults[findCurrent], animate: false)
            pdf.scrollSelectionToVisible(nil)
        }
    }
    func find(index: Int, generation: UInt64, action: String, query: String, sensitive: Bool,
              navigate: Bool, delta: Int, callback: @escaping FindCallback) {
        guard generation >= findGeneration else { return }
        if action == "focus" { if !surface.isHidden { window?.makeFirstResponder(surface) }; return }
        if action == "cancel" { findGeneration = generation; cancelSearch(); return }
        if action == "go" {
            guard generation == findGeneration, !findResults.isEmpty else { return }
            findCurrent = (findCurrent + delta % findResults.count + findResults.count) % findResults.count
            paintFind(navigate: true)
            requestID.withCString { callback($0, index, generation, findResults.count, findCurrent, 0) }; return
        }
        guard action == "search" else { return }
        findGeneration = generation; cancelSearch()
        let emit: (Int32) -> Void = { [weak self] status in
            guard let self, self.findGeneration == generation else { return }
            self.requestID.withCString { callback($0, index, generation, 0, -1, status) }
        }
        if pdfLoad != nil {
            emit(1)
            pendingFind = { [weak self] in self?.find(index: index, generation: generation, action: action,
                query: query, sensitive: sensitive, navigate: navigate, delta: delta, callback: callback) }
            return
        }
        guard pdfActive, let source = pdfSource, pdf.document != nil else { emit(3); return }
        guard !query.isEmpty else { emit(0); return }
        let task = PDFSearchTask(owner: self, index: index, generation: generation, navigate: navigate,
                                 initial: delta, callback: callback)
        searchTask = task; emit(2)
        pdfQueue.async { [weak task] in
            guard let task, !task.ticket.isCancelled else { return }
            let document = autoreleasepool { PDFDocument(data: source) }
            let hasText = document.map { document in
                (0..<document.pageCount).contains { (document.page(at: $0)?.numberOfCharacters ?? 0) > 0 }
            } ?? false
            DispatchQueue.main.async { [weak task] in
                guard let task, !task.ticket.isCancelled, let owner = task.owner, owner.searchTask === task else { return }
                guard let document, !document.isLocked else { task.emit(5); return }
                task.document = document; task.hasText = hasText; document.delegate = task
                document.beginFindString(query, withOptions: sensitive ? [.literal] : [.literal, .caseInsensitive])
            }
        }
    }
    func save() {
        guard let path else { return }
        if pdfActive, let document = pdf.document, let page = pdf.currentPage {
            let point = pdf.convert(CGPoint(x: pdf.bounds.minX, y: pdf.bounds.maxY), to: page)
            pdfStates[path] = PDFState(page: document.index(for: page), point: point,
                                      scale: pdf.scaleFactor, automatic: pdf.autoScales)
        } else if !pdfActive, let state = view.displayState { states[path] = state }
    }
    func webView(in root: NSView) -> NSView? {
        if root.isKind(of: NSClassFromString("WKWebView") ?? NSView.self) { return root }
        for child in root.subviews where !(child is QLPreviewView) && child !== pdf && child !== spinner { if let found = webView(in: child) { return found } }
        return nil
    }
    func hide() {
        cancelSearch()
        save()
        pdfLoad?.cancel(); pdfLoad = nil
        spinner.stopAnimation(nil); spinner.isHidden = true
        // Return native keyboard focus to the WebView before hiding its responder subtree.
        if let responder = window?.firstResponder as? NSView,
           responder === surface || responder.isDescendant(of: surface) {
            let webview = window?.contentView.flatMap { webView(in: $0) }
            window?.makeFirstResponder(webview)
        }
        view.isHidden = true
        pdf.isHidden = true
        pdf.document = nil
        pdfSource = nil
        if view === defaultView { view.previewItem = nil }
        path = nil
    }
    func show(path: String, frame: NSRect, bareEnter: Bool) {
        submitWithBareEnter = bareEnter
        let destination = self.path == path && pdfActive ? pdf.currentDestination : nil
        view.frame = frame
        pdf.frame = frame
        if let destination { pdf.go(to: destination) }
        spinner.frame = NSRect(x: frame.midX - 16, y: frame.midY - 16, width: 32, height: 32)
        if self.path != path {
            cancelSearch()
            pdfSource = nil
            save()
            self.path = path
            // PDFKit provides stable page/zoom restoration without racing Quick Look's
            // asynchronous remote preview. Locked PDFs retain Quick Look's password UI.
            pdfLoad?.cancel(); pdfLoad = nil
            spinner.stopAnimation(nil); spinner.isHidden = true
            if path.lowercased().hasSuffix(".pdf") {
                pdfActive = true
                if view === defaultView { view.previewItem = nil }
                pdf.document = nil
                let ticket = PDFLoadTicket()
                pdfLoad = ticket
                spinner.isHidden = false
                spinner.startAnimation(nil)
                pdfQueue.async { [weak self] in
                    guard !ticket.isCancelled else { return }
                    let source: Data? = autoreleasepool {
                        guard let handle = try? FileHandle(forReadingFrom: URL(fileURLWithPath: path)) else { return nil }
                        defer { try? handle.close() }
                        guard let data = try? handle.read(upToCount: 100 * 1024 * 1024 + 1),
                              data.count <= 100 * 1024 * 1024 else { return nil }
                        return data
                    }
                    let document = source.flatMap { PDFDocument(data: $0) }
                    DispatchQueue.main.async { [weak self] in
                        guard let self, !ticket.isCancelled, self.pdfLoad === ticket,
                              self.path == path else { return }
                        self.pdfLoad = nil
                        self.spinner.stopAnimation(nil); self.spinner.isHidden = true
                        if let document, !document.isLocked {
                            self.pdfSource = source
                            self.pdf.document = document
                            self.pdf.autoScales = self.pdfStates[path]?.automatic ?? true
                            self.pdf.layoutDocumentView()
                            if let state = self.pdfStates[path], let page = document.page(at: state.page) {
                                if !state.automatic { self.pdf.scaleFactor = state.scale }
                                self.pdf.go(to: PDFDestination(page: page, at: state.point))
                            }
                            let pending = self.pendingFind; self.pendingFind = nil; pending?()
                        } else {
                            self.pdfSource = nil
                            self.pdfActive = false
                            self.pdf.isHidden = true
                            self.selectQL(path: path)
                            let pending = self.pendingFind; self.pendingFind = nil; pending?()
                        }
                    }
                }
            } else {
                pdfActive = false
                pdf.document = nil
                selectQL(path: path)
            }
        }
        view.isHidden = pdfActive
        pdf.isHidden = !pdfActive
    }
    func selectQL(path: String) {
        view.isHidden = true
        defaultView.isHidden = true
        let ext = URL(fileURLWithPath: path).pathExtension.lowercased()
        let cacheable = ["doc", "docx", "xls", "xlsx", "ppt", "pptx", "pages", "numbers", "key", "rtf", "odt", "ods", "odp"].contains(ext)
        if cacheable {
            // Some system extensions do not implement displayState. Keep at most two static
            // document views so their native scroll/selection survives switching, without
            // retaining playing media or growing an unbounded collection of remote previews.
            if let index = cachedViews.firstIndex(where: { $0.0 == path }) {
                let cached = cachedViews.remove(at: index)
                cachedViews.append(cached)
                view = cached.1
            } else {
                let created = QLPreviewView(frame: pdf.frame, style: .normal)!
                created.autostarts = false
                created.shouldCloseWithWindow = true
                window.flatMap { popupPreviewParent($0) }?.addSubview(created)
                created.previewItem = URL(fileURLWithPath: path) as NSURL
                cachedViews.append((path, created)); view = created
                if cachedViews.count > 2 {
                    let expired = cachedViews.removeFirst().1
                    expired.close(); expired.removeFromSuperview()
                }
            }
            defaultView.previewItem = nil
        } else {
            view = defaultView
            view.previewItem = URL(fileURLWithPath: path) as NSURL
            if let state = states[path] { view.displayState = state }
        }
        view.frame = pdf.frame
        view.isHidden = false
    }
    func close() {
        hide()
        if let monitor { NSEvent.removeMonitor(monitor); self.monitor = nil }
        if let interactionMonitor { NSEvent.removeMonitor(interactionMonitor); self.interactionMonitor = nil }
        defaultView.close(); defaultView.removeFromSuperview()
        for (_, cached) in cachedViews { cached.close(); cached.removeFromSuperview() }
        cachedViews.removeAll()
        pdf.removeFromSuperview()
        spinner.removeFromSuperview()
        states.removeAll()
        pdfStates.removeAll()
    }
}

// The native surfaces share the WebView's parent and follow its live edge resize directly.
// Freeze them before reserving an offscreen viewport for a pane animation.
func popupPreviewResizing(_ enabled: Bool) {
    guard let preview = nativePreview else { return }
    let mask: NSView.AutoresizingMask = enabled ? [.width, .height] : []
    preview.defaultView.autoresizingMask = mask
    preview.pdf.autoresizingMask = mask
    for (_, cached) in preview.cachedViews { cached.autoresizingMask = mask }
    preview.spinner.autoresizingMask = enabled ? [.minXMargin, .maxXMargin, .minYMargin, .maxYMargin] : []
}

// All view operations are called on Tauri's main thread. Coordinates are CSS points from the
// top-left of the content area; native AppKit coordinates have their origin at the bottom.
@_cdecl("ah_preview_update_view")
func updatePreviewView(_ windowPointer: UnsafeMutableRawPointer, _ request: UnsafePointer<CChar>, _ path: UnsafePointer<CChar>?,
                       _ x: Double, _ y: Double, _ width: Double, _ height: Double,
                       _ bareEnter: Bool, _ keyCallback: @escaping KeyCallback) -> Bool {
    let window = Unmanaged<NSWindow>.fromOpaque(windowPointer).takeUnretainedValue()
    guard let content = popupPreviewParent(window) else { return false }
    let requestID = String(cString: request)
    let path = path.map { String(cString: $0) }
    // A queued DOM measurement must not undo an AppKit live-resize frame with an older
    // width or height. The merged preview is anchored to the canvas's right and bottom.
    let responsive = popupCanvasIsResponsive(window)
    let frame = NSRect(x: x, y: content.isFlipped ? y : content.bounds.height - y - height,
                       width: responsive ? max(0, content.bounds.width - x) : width,
                       height: responsive ? max(0, content.bounds.height - y) : height)
    applyPreviewUpdate(window: window, requestID: requestID, path: path,
                       frame: frame, bareEnter: bareEnter, keyCallback: keyCallback)
    return path == nil || nativePreview != nil
}

private func applyPreviewUpdate(window: NSWindow, requestID: String, path: String?, frame: NSRect,
                                bareEnter: Bool, keyCallback: @escaping KeyCallback) {
    if let previous = nativePreview, previous.window !== window || previous.requestID != requestID {
        previous.close()
        nativePreview = nil
    }
    guard let path else { nativePreview?.hide(); return }
    if nativePreview == nil {
        nativePreview = EmbeddedPreview(window: window, requestID: requestID, keyCallback: keyCallback)
    }
    nativePreview?.show(path: path, frame: frame, bareEnter: bareEnter)
    popupPreviewResizing(popupCanvasIsResponsive(window))
}

// The explicit Dev review uses this read-only coordinate to verify native preview anchoring.
@_cdecl("ah_preview_screen_x")
func previewScreenX() -> Double {
    guard let preview = nativePreview, !preview.surface.isHidden, let window = preview.window else { return .nan }
    let frame = preview.surface.convert(preview.surface.bounds, to: nil)
    return window.convertToScreen(frame).minX
}

@_cdecl("ah_preview_find")
func findInPreview(_ request: UnsafePointer<CChar>, _ index: Int, _ generation: UInt64,
                   _ action: UnsafePointer<CChar>, _ query: UnsafePointer<CChar>, _ sensitive: Bool,
                   _ navigate: Bool, _ delta: Int, _ callback: @escaping FindCallback) -> Bool {
    guard let preview = nativePreview, preview.requestID == String(cString: request), !preview.surface.isHidden else { return false }
    preview.find(index: index, generation: generation, action: String(cString: action), query: String(cString: query),
                 sensitive: sensitive, navigate: navigate, delta: delta, callback: callback)
    return true
}
