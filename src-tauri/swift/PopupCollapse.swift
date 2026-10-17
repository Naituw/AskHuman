import AppKit
import ImageIO
import QuartzCore
import UniformTypeIdentifiers

// Prototype-stage native driver for the collapsible popup window (docs/specs/popup-inbox-collapse.md).
// It is only called by `examples/popup_collapse_demo.rs` today; nothing in the product binary
// reaches it. It animates the NSWindow frame frame-by-frame on the main thread, locked to the
// display refresh, while the WKWebView keeps a fixed viewport (PopupCanvas.swift) so the page never
// reflows. Only public AppKit API is used.

// MARK: - cubic-bezier easing (CSS semantics)

private struct CollapseBezier {
    let ax: Double, bx: Double, cx: Double, ay: Double, by: Double, cy: Double

    init(_ x1: Double, _ y1: Double, _ x2: Double, _ y2: Double) {
        cx = 3 * x1; bx = 3 * (x2 - x1) - cx; ax = 1 - cx - bx
        cy = 3 * y1; by = 3 * (y2 - y1) - cy; ay = 1 - cy - by
    }

    private func x(_ t: Double) -> Double { ((ax * t + bx) * t + cx) * t }
    private func y(_ t: Double) -> Double { ((ay * t + by) * t + cy) * t }
    private func dx(_ t: Double) -> Double { (3 * ax * t + 2 * bx) * t + cx }

    func solve(_ progress: Double) -> Double {
        if progress <= 0 { return 0 }
        if progress >= 1 { return 1 }
        var t = progress
        for _ in 0..<8 {
            let error = x(t) - progress
            if abs(error) < 1e-7 { return y(t) }
            let slope = dx(t)
            if abs(slope) < 1e-6 { break }
            t -= error / slope
        }
        var low = 0.0, high = 1.0
        t = progress
        for _ in 0..<40 {
            let value = x(t)
            if abs(value - progress) < 1e-7 { break }
            if value < progress { low = t } else { high = t }
            t = (low + high) / 2
        }
        return y(t)
    }
}

// MARK: - parameters

private struct CollapseParams {
    var toWidth = 0.0
    var toHeight = 0.0
    var toDX = 0.0
    /// Canvas origin (WebKit viewport x relative to the window) at the start / end. Absent = keep the
    /// current origin, which is `-reserve` when the page keeps an offscreen left reserve.
    var canvasX0: Double?
    var canvasX1: Double?
    var durationMs = 0.0
    var curve: [Double] = [0.2, 0, 0, 1]
    var widthStart = 0.0, widthEnd = 1.0
    var heightStart = 0.0, heightEnd = 1.0
    var lights = "keep"
    var lightsStart = 0.0, lightsEnd = 0.4
    var resume = false
    var minWidth = 1.0, minHeight = 1.0
    var resizable: Bool?
    var shadow: Bool?
    /// How a frame reaches the window server (experiments with tearing between window bounds and content):
    /// "baseline", "display" (setFrame display: true), "screens" (disableScreenUpdatesUntilFlush + flush), "both",
    /// "group" (inside an NSAnimationContext grouping), "group+screens".
    var sync = "baseline"

    init(_ json: [String: Any]) {
        func number(_ key: String, _ fallback: Double) -> Double {
            (json[key] as? NSNumber)?.doubleValue ?? fallback
        }
        toWidth = number("toWidth", 0)
        toHeight = number("toHeight", 0)
        toDX = number("toDX", 0)
        canvasX0 = (json["canvasX0"] as? NSNumber)?.doubleValue
        canvasX1 = (json["canvasX1"] as? NSNumber)?.doubleValue
        durationMs = number("durationMs", 0)
        if let values = json["curve"] as? [NSNumber], values.count == 4 { curve = values.map { $0.doubleValue } }
        widthStart = number("widthStart", 0); widthEnd = number("widthEnd", 1)
        heightStart = number("heightStart", 0); heightEnd = number("heightEnd", 1)
        lights = (json["lights"] as? String) ?? "keep"
        lightsStart = number("lightsStart", 0); lightsEnd = number("lightsEnd", 0.4)
        resume = (json["resume"] as? Bool) ?? false
        minWidth = number("minWidth", 1); minHeight = number("minHeight", 1)
        resizable = json["resizable"] as? Bool
        shadow = json["shadow"] as? Bool
        sync = (json["sync"] as? String) ?? "baseline"
    }
}

// Report layout shared with the Rust example (index → meaning).
//  0 frames  1 maxGapMs  2 meanGapMs  3 droppedFrames  4 wallMs  5 finalX  6 finalTop  7 finalW  8 finalH
//  9 cancelled  10 maxErrorPx  11 refreshHz  12 canvasW  13 canvasH  14 fromX  15 fromTop  16 fromW  17 fromH
// 18 p95GapMs
typealias PopupCollapseDone = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<Double>?, Int32) -> Void

private var activeCollapseRun: CollapseRun?

private final class CollapseRun: NSObject {
    let window: NSWindow
    let webview: UnsafeMutableRawPointer
    let params: CollapseParams
    let done: PopupCollapseDone
    let userData: UnsafeMutableRawPointer?
    let curve: CollapseBezier

    var displayLink: AnyObject?
    var fallbackTimer: Timer?
    var origin: CFTimeInterval = -1
    var wallStart = CACurrentMediaTime()
    var from = NSRect.zero
    var to = NSRect.zero
    var canvasWidth = 0.0
    var canvasHeight = 0.0
    /// Resolved canvas origins (see `CollapseParams.canvasX0`).
    var canvasStart = 0.0
    var canvasEnd = 0.0
    var scale = 2.0
    var lightsFrom = 1.0
    var frames = 0
    var gaps: [Double] = []
    /// Main-thread time spent placing each frame (setFrame + canvas + display), in ms.
    var costs: [Double] = []
    /// Per tick: callback start and placement end, in ms after the display link's `timestamp` (the vsync that
    /// woke it). A tick that ends too late misses the compositor and the old size is shown for one more vsync.
    var lates: [Double] = []
    var ends: [Double] = []
    /// Per tick: the display time (ms, mach clock) the placed frame is meant for.
    var targets: [Double] = []
    /// Per tick: the window size placed (points).
    var placedWidths: [Double] = []
    var placedHeights: [Double] = []
    var startDelayMs = 0.0
    var lastStamp = 0.0
    var interval = 1.0 / 60.0
    var maxError = 0.0
    var finished = false

    init(window: NSWindow, webview: UnsafeMutableRawPointer, params: CollapseParams,
         done: PopupCollapseDone, userData: UnsafeMutableRawPointer?) {
        self.window = window
        self.webview = webview
        self.params = params
        self.done = done
        self.userData = userData
        self.curve = CollapseBezier(params.curve[0], params.curve[1], params.curve[2], params.curve[3])
    }

    private var view: NSView { Unmanaged<NSView>.fromOpaque(webview).takeUnretainedValue() }

    private func snap(_ value: Double) -> Double { (value * scale).rounded() / scale }

    private func setLights(alpha: Double, hidden: Bool) {
        let kinds: [NSWindow.ButtonType] = [.closeButton, .miniaturizeButton, .zoomButton]
        for kind in kinds {
            guard let button = window.standardWindowButton(kind) else { continue }
            button.alphaValue = CGFloat(alpha)
            button.isHidden = hidden
        }
    }

    /// Final frame: top-left anchored, shifted horizontally by `toDX`, then kept on the visible screen.
    private func target(from: NSRect) -> NSRect {
        var width = params.toWidth > 0 ? params.toWidth : from.width
        var height = params.toHeight > 0 ? params.toHeight : from.height
        var x = from.minX + params.toDX
        var top = from.maxY
        if let visible = (window.screen ?? NSScreen.main)?.visibleFrame {
            width = min(width, visible.width)
            height = min(height, visible.height)
            x = min(max(x, visible.minX), max(visible.minX, visible.maxX - width))
            top = min(max(top, visible.minY + height), visible.maxY)
        }
        return NSRect(x: snap(x), y: snap(top - height), width: snap(width), height: snap(height))
    }

    func begin() {
        scale = Double(window.backingScaleFactor)
        from = window.frame
        to = target(from: from)

        // Safety net: the caller normally pins the canvas first (ah_collapse_prepare).
        let content = view
        if !(content.superview is PopupCanvas) || popupCanvasIsResponsive(window) {
            // Freeze the viewport as it is, including any offscreen left reserve (canvas x < 0).
            let reserve = -Double((content.superview as? PopupCanvas)?.frame.minX ?? 0)
            preparePopupCanvas(webview, reserve, Double(content.frame.width), Double(content.frame.height), 0)
        }
        canvasWidth = Double(content.frame.width)
        canvasHeight = Double(content.frame.height)
        let currentCanvasX = Double(content.superview?.frame.minX ?? 0)
        canvasStart = params.canvasX0 ?? currentCanvasX
        canvasEnd = params.canvasX1 ?? currentCanvasX

        window.minSize = NSSize(width: 1, height: 1)
        window.contentMinSize = NSSize(width: 1, height: 1)
        if params.resizable == true { window.styleMask.insert(.resizable) }
        if let shadow = params.shadow { window.hasShadow = shadow }

        lightsFrom = Double(window.standardWindowButton(.closeButton)?.alphaValue ?? 1)
        if params.lights == "show" {
            // Begin from the current alpha (0 after a full hide); an interrupted fade-out resumes from where it was.
            let hiddenNow = window.standardWindowButton(.closeButton)?.isHidden ?? true
            lightsFrom = hiddenNow ? 0 : lightsFrom
            setLights(alpha: lightsFrom, hidden: false)
        }

        if params.durationMs <= 0 {
            frames = 1
            place(to, canvasX: canvasEnd, lightsAlpha: nil)
            complete()
            return
        }
        startClock()
    }

    private func startClock() {
        if #available(macOS 14.0, *) {
            let link = window.displayLink(target: self, selector: #selector(linkTick(_:)))
            link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
            link.add(to: .main, forMode: .common)
            displayLink = link
        } else {
            let timer = Timer(timeInterval: 1.0 / 120.0, repeats: true) { [weak self] _ in
                guard let self else { return }
                let now = CACurrentMediaTime()
                self.step(target: now, stamp: now)
            }
            RunLoop.main.add(timer, forMode: .common)
            fallbackTimer = timer
        }
    }

    private func stopClock() {
        if #available(macOS 14.0, *), let link = displayLink as? CADisplayLink { link.invalidate() }
        displayLink = nil
        fallbackTimer?.invalidate()
        fallbackTimer = nil
    }

    @available(macOS 14.0, *)
    @objc private func linkTick(_ link: CADisplayLink) {
        interval = max(0.001, link.targetTimestamp - link.timestamp)
        step(target: link.targetTimestamp, stamp: link.timestamp)
    }

    private func axis(_ t: Double, _ start: Double, _ end: Double) -> Double {
        if end <= start { return t >= end ? 1 : 0 }
        return curve.solve(min(1, max(0, (t - start) / (end - start))))
    }

    private func frame(at t: Double) -> (rect: NSRect, canvasX: Double) {
        let width = snap(from.width + (to.width - from.width) * axis(t, params.widthStart, params.widthEnd))
        let height = snap(from.height + (to.height - from.height) * axis(t, params.heightStart, params.heightEnd))
        let progress = curve.solve(t)
        let x = snap(from.minX + (to.minX - from.minX) * progress)
        let top = snap(from.maxY + (to.maxY - from.maxY) * progress)
        let canvasX = snap(canvasStart + (canvasEnd - canvasStart) * progress)
        return (NSRect(x: x, y: top - height, width: width, height: height), canvasX)
    }

    private func lightsAlpha(at t: Double) -> Double? {
        let span = max(params.lightsEnd - params.lightsStart, 0.0001)
        let p = min(1, max(0, (t - params.lightsStart) / span))
        switch params.lights {
        case "hide": return lightsFrom * (1 - p)
        case "show": return lightsFrom + (1 - lightsFrom) * p
        default: return nil
        }
    }

    private func place(_ rect: NSRect, canvasX: Double, lightsAlpha: Double?) {
        let holdScreen = params.sync == "screens" || params.sync == "both" || params.sync == "group+screens"
        let displayNow = params.sync == "display" || params.sync == "both"
        let grouped = params.sync == "group" || params.sync == "group+screens"
        if holdScreen { window.disableScreenUpdatesUntilFlush() }
        if grouped {
            NSAnimationContext.beginGrouping()
            NSAnimationContext.current.duration = 0
            NSAnimationContext.current.allowsImplicitAnimation = false
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        window.setFrame(rect, display: displayNow)
        // The viewport extent stays fixed; its top edge follows the window's top edge. A non-zero
        // `canvasX` slides the viewport sideways (the Sidebar appearing keeps the body in place).
        positionPopupCanvas(webview, canvasX, canvasHeight)
        if let alpha = lightsAlpha { setLights(alpha: alpha, hidden: false) }
        window.displayIfNeeded()
        CATransaction.commit()
        if grouped { NSAnimationContext.endGrouping() }
        CATransaction.flush()
        if holdScreen { window.flush() }
        let actual = window.frame
        maxError = max(maxError, abs(actual.minX - rect.minX), abs(actual.maxY - rect.maxY),
                       abs(actual.width - rect.width), abs(actual.height - rect.height))
    }

    private func step(target: CFTimeInterval, stamp: CFTimeInterval) {
        let entered = CACurrentMediaTime()
        if finished { return }
        if origin < 0 {
            origin = target
            startDelayMs = (CACurrentMediaTime() - wallStart) * 1000
        }
        if lastStamp > 0 { gaps.append((stamp - lastStamp) * 1000) }
        lastStamp = stamp
        frames += 1
        let t = max(0, target - origin) * 1000 / params.durationMs
        if t >= 1 { complete(); return }
        let next = frame(at: t)
        let began = CACurrentMediaTime()
        place(next.rect, canvasX: next.canvasX, lightsAlpha: lightsAlpha(at: t))
        let ended = CACurrentMediaTime()
        costs.append((ended - began) * 1000)
        lates.append((entered - stamp) * 1000)
        ends.append((ended - stamp) * 1000)
        targets.append(target * 1000)
        placedWidths.append(Double(next.rect.width))
        placedHeights.append(Double(next.rect.height))
    }

    private func complete() {
        if finished { return }
        finished = true
        stopClock()
        if params.durationMs > 0 { place(to, canvasX: canvasEnd, lightsAlpha: nil) }
        switch params.lights {
        // "settle" fades nothing but guarantees the final state (a resize that interrupted a fade-out).
        case "hide", "settle": setLights(alpha: 1, hidden: true)
        case "show": setLights(alpha: 1, hidden: false)
        default: break
        }
        if params.resizable == false { window.styleMask.remove(.resizable) }
        if params.resume {
            // The viewport keeps the same offscreen left reserve it had during the run.
            resumePopupCanvas(webview, -canvasEnd, 0, params.minWidth, params.minHeight)
            window.contentMinSize = NSSize(width: params.minWidth, height: params.minHeight)
        }
        report(cancelled: false)
    }

    func cancel() {
        if finished { return }
        finished = true
        stopClock()
        report(cancelled: true)
    }

    private func report(cancelled: Bool) {
        let actual = window.frame
        let mean = gaps.isEmpty ? 0 : gaps.reduce(0, +) / Double(gaps.count)
        let sorted = gaps.sorted()
        let p95 = sorted.isEmpty ? 0 : sorted[min(sorted.count - 1, Int(Double(sorted.count) * 0.95))]
        let budget = interval * 1000
        var dropped = 0
        for gap in gaps where gap > budget * 1.5 { dropped += max(1, Int((gap / budget).rounded()) - 1) }
        var values: [Double] = []
        values.append(Double(frames))
        values.append(gaps.max() ?? 0)
        values.append(mean)
        values.append(Double(dropped))
        values.append((CACurrentMediaTime() - wallStart) * 1000)
        values.append(Double(actual.minX))
        values.append(Double(actual.maxY))
        values.append(Double(actual.width))
        values.append(Double(actual.height))
        values.append(cancelled ? 1 : 0)
        values.append(maxError)
        values.append(1 / interval)
        values.append(canvasWidth)
        values.append(canvasHeight)
        values.append(Double(from.minX))
        values.append(Double(from.maxY))
        values.append(Double(from.width))
        values.append(Double(from.height))
        values.append(p95)
        values.append(startDelayMs)
        values.append(costs.max() ?? 0)
        values.append(costs.isEmpty ? 0 : costs.reduce(0, +) / Double(costs.count))
        // Per-tick series (first 80 ticks): gap since the previous tick, callback lateness, placement end.
        let ticks = min(80, lates.count)
        values.append(Double(ticks))
        values.append(0)
        values.append(contentsOf: gaps.prefix(max(0, ticks - 1)))
        values.append(contentsOf: lates.prefix(ticks))
        values.append(contentsOf: ends.prefix(ticks))
        values.append(contentsOf: targets.prefix(ticks))
        values.append(contentsOf: placedWidths.prefix(ticks))
        values.append(contentsOf: placedHeights.prefix(ticks))
        if activeCollapseRun === self { activeCollapseRun = nil }
        values.withUnsafeBufferPointer { done(userData, $0.baseAddress, Int32(values.count)) }
    }
}

// MARK: - C entry points

@_cdecl("ah_collapse_animate")
func popupCollapseAnimate(_ webview: UnsafeMutableRawPointer, _ windowPointer: UnsafeMutableRawPointer,
                          _ json: UnsafePointer<CChar>, _ done: PopupCollapseDone,
                          _ userData: UnsafeMutableRawPointer?) {
    let window = Unmanaged<NSWindow>.fromOpaque(windowPointer).takeUnretainedValue()
    let object = (try? JSONSerialization.jsonObject(with: Data(String(cString: json).utf8))) as? [String: Any] ?? [:]
    // A newer request interrupts the running one and continues from the actual current frame.
    activeCollapseRun?.cancel()
    let run = CollapseRun(window: window, webview: webview, params: CollapseParams(object), done: done, userData: userData)
    activeCollapseRun = run
    run.begin()
}

@_cdecl("ah_collapse_prepare")
func popupCollapsePrepare(_ webview: UnsafeMutableRawPointer, _ left: Double, _ width: Double,
                          _ height: Double, _ visibleLeft: Double) {
    preparePopupCanvas(webview, left, width, height, visibleLeft)
}

@_cdecl("ah_collapse_resume")
func popupCollapseResume(_ webview: UnsafeMutableRawPointer, _ windowPointer: UnsafeMutableRawPointer,
                         _ left: Double, _ visibleLeft: Double, _ minWidth: Double, _ minHeight: Double) {
    let window = Unmanaged<NSWindow>.fromOpaque(windowPointer).takeUnretainedValue()
    resumePopupCanvas(webview, left, visibleLeft, minWidth, minHeight)
    window.contentMinSize = NSSize(width: minWidth, height: minHeight)
}

/// Raise without activating the application or taking key focus (same contract as an inbox arrival).
@_cdecl("ah_collapse_front")
func popupCollapseFront(_ windowPointer: UnsafeMutableRawPointer) {
    let window = Unmanaged<NSWindow>.fromOpaque(windowPointer).takeUnretainedValue()
    if window.isMiniaturized { window.deminiaturize(nil) }
    window.orderFrontRegardless()
}

/// Silent test mode: the process refuses to activate itself, so a scripted run can never take the keyboard focus from
/// whoever is using the machine. tao activates the app at launch and in `set_focus`, both through
/// `-[NSApplication activateIgnoringOtherApps:]`; windows are still shown (and ordered front) but stay inactive.
@_cdecl("ah_collapse_forbid_activation")
func popupCollapseForbidActivation() {
    guard let method = class_getInstanceMethod(NSApplication.self, NSSelectorFromString("activateIgnoringOtherApps:")) else {
        return
    }
    let refuse: @convention(block) (AnyObject, Bool) -> Void = { _, _ in }
    method_setImplementation(method, imp_implementationWithBlock(refuse))
    if let modern = class_getInstanceMethod(NSApplication.self, NSSelectorFromString("activate")) {
        let refuseModern: @convention(block) (AnyObject) -> Void = { _ in }
        method_setImplementation(modern, imp_implementationWithBlock(refuseModern))
    }
}

/// Test hook for "the user left AskHuman": hands the activation to Finder, like a click on another app would.
/// Returns 1 when macOS accepted the request.
@_cdecl("ah_collapse_activate_other")
func popupCollapseActivateOther() -> Int32 {
    guard let other = NSWorkspace.shared.runningApplications.first(where: { $0.bundleIdentifier == "com.apple.finder" }) else {
        return 0
    }
    if #available(macOS 14.0, *) {
        NSApp.yieldActivation(to: other)
        return other.activate(from: NSRunningApplication.current, options: []) ? 1 : 0
    }
    return other.activate(options: []) ? 1 : 0
}

// Info layout (index → meaning), written into `out` (capacity ≥ 24):
//  0 x  1 top  2 width  3 height  4 scale  5 screenX  6 screenTop  7 screenW  8 screenH  9 level
// 10 minW 11 minH 12 lightsHidden 13 resizable 14 canvasInstalled 15 canvasResponsive 16 canvasX
// 17 viewW 18 viewH 19 visible 20 key 21 lightsAlpha 22 hasShadow
@_cdecl("ah_collapse_info")
func popupCollapseInfo(_ webview: UnsafeMutableRawPointer, _ windowPointer: UnsafeMutableRawPointer,
                       _ out: UnsafeMutablePointer<Double>) {
    let window = Unmanaged<NSWindow>.fromOpaque(windowPointer).takeUnretainedValue()
    let view = Unmanaged<NSView>.fromOpaque(webview).takeUnretainedValue()
    let frame = window.frame
    let screen = (window.screen ?? NSScreen.main)?.visibleFrame ?? .zero
    let close = window.standardWindowButton(.closeButton)
    let canvas = view.superview as? PopupCanvas
    var values: [Double] = []
    values.append(Double(frame.minX))
    values.append(Double(frame.maxY))
    values.append(Double(frame.width))
    values.append(Double(frame.height))
    values.append(Double(window.backingScaleFactor))
    values.append(Double(screen.minX))
    values.append(Double(screen.maxY))
    values.append(Double(screen.width))
    values.append(Double(screen.height))
    values.append(Double(window.level.rawValue))
    values.append(Double(window.minSize.width))
    values.append(Double(window.minSize.height))
    values.append((close?.isHidden ?? true) ? 1 : 0)
    values.append(window.styleMask.contains(.resizable) ? 1 : 0)
    values.append(canvas == nil ? 0 : 1)
    values.append(popupCanvasIsResponsive(window) ? 1 : 0)
    values.append(Double(canvas?.frame.minX ?? 0))
    values.append(Double(view.frame.width))
    values.append(Double(view.frame.height))
    values.append(window.isVisible ? 1 : 0)
    values.append(window.isKeyWindow ? 1 : 0)
    values.append(Double(close?.alphaValue ?? 1))
    values.append(window.hasShadow ? 1 : 0)
    for (index, value) in values.enumerated() { out[index] = value }
}

// MARK: - Window capture (prototype verification only)
//
// Captures this process's own window as the window server composited it (what the user sees,
// including WebKit content and the material). Capturing an app's own windows needs no screen
// recording permission. `CGWindowListCreateImage` is deprecated, so it is resolved at run time.

private typealias CollapseWindowListImage = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?

private let collapseWindowListImage: CollapseWindowListImage? = {
    guard let handle = dlopen("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics", RTLD_LAZY),
          let symbol = dlsym(handle, "CGWindowListCreateImage") else { return nil }
    return unsafeBitCast(symbol, to: CollapseWindowListImage.self)
}()

private struct CollapseShot {
    let image: CGImage
    /// Window frame (AppKit screen coordinates) at capture time.
    let frame: NSRect
    let scale: Double
}

private var collapseShots: [String: CollapseShot] = [:]

private func collapseRGBA(_ image: CGImage) -> [UInt8]? {
    let width = image.width, height = image.height
    guard width > 0, height > 0 else { return nil }
    var bytes = [UInt8](repeating: 0, count: width * height * 4)
    let space = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
    let drawn = bytes.withUnsafeMutableBytes { buffer -> Bool in
        guard let context = CGContext(data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: width * 4, space: space,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return true
    }
    return drawn ? bytes : nil
}

/// Crop `region` (AppKit screen coordinates; `top` is the region's top edge) out of a stored shot.
private func collapseCrop(_ shot: CollapseShot, x: Double, top: Double, width: Double, height: Double) -> CGImage? {
    let scale = shot.scale
    let rect = CGRect(x: ((x - Double(shot.frame.minX)) * scale).rounded(),
                      y: ((Double(shot.frame.maxY) - top) * scale).rounded(),
                      width: (width * scale).rounded(), height: (height * scale).rounded())
    let bounds = CGRect(x: 0, y: 0, width: shot.image.width, height: shot.image.height)
    guard bounds.contains(rect) else { return nil }
    return shot.image.cropping(to: rect)
}

/// Capture `window` under `label`; optionally also write it as a PNG. Returns 0 on success.
@_cdecl("ah_collapse_capture")
func popupCollapseCapture(_ windowPointer: UnsafeMutableRawPointer, _ label: UnsafePointer<CChar>,
                          _ path: UnsafePointer<CChar>?) -> Int32 {
    let window = Unmanaged<NSWindow>.fromOpaque(windowPointer).takeUnretainedValue()
    guard let capture = collapseWindowListImage else { return 1 }
    // kCGWindowListOptionIncludingWindow = 1 << 3, kCGWindowImageBoundsIgnoreFraming = 1 << 0 (no shadow)
    guard let image = capture(CGRect.null, 1 << 3, UInt32(window.windowNumber), 1)?.takeRetainedValue() else { return 2 }
    collapseShots[String(cString: label)] = CollapseShot(image: image, frame: window.frame, scale: Double(window.backingScaleFactor))
    if let path {
        let data = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
        do { try data?.write(to: URL(fileURLWithPath: String(cString: path))) } catch { return 3 }
    }
    return 0
}

/// Compare the same screen-space region in two stored captures.
/// out: 0 mean |Δ| per channel (0–255), 1 fraction of pixels differing by more than 24 in any channel,
/// 2 luma standard deviation of A, 3 of B, 4 region width px, 5 height px. Returns 0 on success.
@_cdecl("ah_collapse_diff")
func popupCollapseDiff(_ labelA: UnsafePointer<CChar>, _ labelB: UnsafePointer<CChar>,
                       _ x: Double, _ top: Double, _ width: Double, _ height: Double,
                       _ out: UnsafeMutablePointer<Double>) -> Int32 {
    guard let a = collapseShots[String(cString: labelA)], let b = collapseShots[String(cString: labelB)] else { return 1 }
    guard let cropA = collapseCrop(a, x: x, top: top, width: width, height: height),
          let cropB = collapseCrop(b, x: x, top: top, width: width, height: height),
          cropA.width == cropB.width, cropA.height == cropB.height,
          let bytesA = collapseRGBA(cropA), let bytesB = collapseRGBA(cropB) else { return 2 }
    let pixels = cropA.width * cropA.height
    var total = 0.0, changed = 0
    var sumA = 0.0, sumSquaresA = 0.0, sumB = 0.0, sumSquaresB = 0.0
    for p in 0..<pixels {
        let i = p * 4
        var worst = 0
        for c in 0..<3 {
            let d = abs(Int(bytesA[i + c]) - Int(bytesB[i + c]))
            total += Double(d)
            worst = max(worst, d)
        }
        if worst > 24 { changed += 1 }
        let lumaA = 0.299 * Double(bytesA[i]) + 0.587 * Double(bytesA[i + 1]) + 0.114 * Double(bytesA[i + 2])
        let lumaB = 0.299 * Double(bytesB[i]) + 0.587 * Double(bytesB[i + 1]) + 0.114 * Double(bytesB[i + 2])
        sumA += lumaA; sumSquaresA += lumaA * lumaA
        sumB += lumaB; sumSquaresB += lumaB * lumaB
    }
    let n = Double(max(pixels, 1))
    out[0] = total / (n * 3)
    out[1] = Double(changed) / n
    out[2] = max(0, sumSquaresA / n - (sumA / n) * (sumA / n)).squareRoot()
    out[3] = max(0, sumSquaresB / n - (sumB / n) * (sumB / n)).squareRoot()
    out[4] = Double(cropA.width)
    out[5] = Double(cropA.height)
    return 0
}

// MARK: - Backdrop (prototype verification only)
//
// A plain colour window right behind the answer window. A screen recording of that region (taken from outside this
// process, see `scripts/popup-collapse-demo.sh --video`) then shows nothing but the answer window on a known colour,
// which makes its outline easy to measure frame by frame.

private var collapseBackdrop: NSWindow?

/// Show (`enable` != 0) or remove the backdrop. The region is in points with a top-left origin on the main display.
@_cdecl("ah_collapse_backdrop")
func popupCollapseBackdrop(_ windowPointer: UnsafeMutableRawPointer, _ enable: Int32,
                           _ x: Double, _ y: Double, _ width: Double, _ height: Double) {
    let window = Unmanaged<NSWindow>.fromOpaque(windowPointer).takeUnretainedValue()
    if enable == 0 {
        collapseBackdrop?.orderOut(nil)
        collapseBackdrop = nil
        return
    }
    let screenHeight = Double(NSScreen.screens.first?.frame.height ?? 0)
    let rect = NSRect(x: x, y: screenHeight - y - height, width: width, height: height)
    let backdrop = collapseBackdrop ?? {
        let created = NSWindow(contentRect: rect, styleMask: [.borderless], backing: .buffered, defer: false)
        created.isReleasedWhenClosed = false
        created.isOpaque = true
        created.hasShadow = false
        created.ignoresMouseEvents = true
        created.animationBehavior = .none
        created.collectionBehavior = [.stationary, .ignoresCycle, .fullScreenAuxiliary]
        return created
    }()
    // Magenta: no channel of the (grey) answer window or its shadow matches it, so the window is "anything with green".
    backdrop.backgroundColor = NSColor(srgbRed: 1, green: 0, blue: 1, alpha: 1)
    backdrop.level = window.level
    backdrop.setFrame(rect, display: true)
    backdrop.order(.below, relativeTo: window.windowNumber)
    collapseBackdrop = backdrop
}

// MARK: - Control animations (prototype verification only)
//
// Reference animations on a plain window, recorded the same way as the real one, to tell whether a frame hitch comes
// from this app's window / page or from the window server and the recorder:
//   "driver"  the same per-display-link-tick `setFrame` loop the real driver uses,
//   "appkit"  AppKit's own window animation (`animator().setFrame`),
//   "layer"   a constant window whose content layer is animated by Core Animation (renders at vsync in the server).

typealias PopupCollapseControlDone = @convention(c) (UnsafeMutableRawPointer?) -> Void

private var collapseControlKeepAlive: [AnyObject] = []

private final class ControlDriver: NSObject {
    let window: NSWindow
    let from: NSRect
    let to: NSRect
    let duration: Double
    let curve = CollapseBezier(0.2, 0, 0, 1)
    let done: () -> Void
    var link: AnyObject?
    var origin = -1.0
    var finished = false

    init(window: NSWindow, from: NSRect, to: NSRect, duration: Double, done: @escaping () -> Void) {
        self.window = window; self.from = from; self.to = to; self.duration = duration; self.done = done
    }

    func start() {
        guard #available(macOS 14.0, *) else { done(); return }
        let displayLink = window.displayLink(target: self, selector: #selector(tick(_:)))
        displayLink.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
        displayLink.add(to: .main, forMode: .common)
        link = displayLink
    }

    private func place(_ rect: NSRect) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        window.setFrame(rect, display: false)
        window.displayIfNeeded()
        CATransaction.commit()
        CATransaction.flush()
    }

    @available(macOS 14.0, *)
    @objc private func tick(_ displayLink: CADisplayLink) {
        if finished { return }
        if origin < 0 { origin = displayLink.targetTimestamp }
        let t = (displayLink.targetTimestamp - origin) / duration
        if t >= 1 {
            finished = true
            displayLink.invalidate()
            place(to)
            done()
            return
        }
        let p = curve.solve(max(0, t))
        let scale = Double(window.backingScaleFactor)
        func snap(_ value: Double) -> Double { (value * scale).rounded() / scale }
        let width = snap(from.width + (to.width - from.width) * p)
        let height = snap(from.height + (to.height - from.height) * p)
        place(NSRect(x: from.minX, y: from.maxY - height, width: width, height: height))
    }
}

private func collapseControlWindow(frame: NSRect, level: NSWindow.Level, layer: Bool) -> (NSWindow, CALayer) {
    let window = NSWindow(contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.animationBehavior = .none
    window.level = level
    window.collectionBehavior = [.stationary, .ignoresCycle, .fullScreenAuxiliary]
    window.isOpaque = false
    window.backgroundColor = .clear
    window.hasShadow = !layer
    let content = NSView(frame: NSRect(origin: .zero, size: frame.size))
    content.wantsLayer = true
    window.contentView = content
    let box = CALayer()
    box.backgroundColor = NSColor(srgbRed: 0.24, green: 0.24, blue: 0.26, alpha: 1).cgColor
    box.cornerRadius = 12
    if layer {
        box.anchorPoint = CGPoint(x: 0, y: 1)
        box.bounds = CGRect(origin: .zero, size: frame.size)
        box.position = CGPoint(x: 0, y: frame.height)
        content.layer?.addSublayer(box)
    } else {
        box.frame = content.bounds
        box.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
        content.layer?.addSublayer(box)
        content.layer?.cornerRadius = 12
        content.layer?.masksToBounds = true
    }
    return (window, box)
}

/// Run `cycles` collapse / expand cycles of a reference animation on a plain window that replaces `windowPointer`
/// while it runs. Sizes in points. Calls `done` when finished.
@_cdecl("ah_collapse_control")
func popupCollapseControl(_ windowPointer: UnsafeMutableRawPointer, _ mode: UnsafePointer<CChar>, _ cycles: Int32,
                          _ restMs: Int32, _ fromWidth: Double, _ fromHeight: Double, _ toWidth: Double,
                          _ toHeight: Double, _ done: PopupCollapseControlDone, _ userData: UnsafeMutableRawPointer?) {
    let popup = Unmanaged<NSWindow>.fromOpaque(windowPointer).takeUnretainedValue()
    let kind = String(cString: mode)
    let start = popup.frame
    let big = NSRect(x: start.minX, y: start.maxY - fromHeight, width: fromWidth, height: fromHeight)
    let small = NSRect(x: start.minX, y: start.maxY - toHeight, width: toWidth, height: toHeight)
    let (window, box) = collapseControlWindow(frame: big, level: popup.level, layer: kind == "layer")
    popup.orderOut(nil)
    window.orderFrontRegardless()
    collapseControlKeepAlive = [window, box]
    let collapseSeconds = 0.44, expandSeconds = 0.48
    let timing = CAMediaTimingFunction(controlPoints: 0.2, 0, 0, 1)

    func finish() {
        window.orderOut(nil)
        collapseControlKeepAlive = []
        popup.orderFrontRegardless()
        done(userData)
    }
    func transition(to target: NSRect, from origin: NSRect, seconds: Double, completion: @escaping () -> Void) {
        switch kind {
        case "appkit":
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = seconds
                context.timingFunction = timing
                context.allowsImplicitAnimation = true
                window.animator().setFrame(target, display: true)
            }, completionHandler: completion)
        case "layer":
            let animation = CABasicAnimation(keyPath: "bounds")
            animation.fromValue = CGRect(origin: .zero, size: origin.size)
            animation.toValue = CGRect(origin: .zero, size: target.size)
            animation.duration = seconds
            animation.timingFunction = timing
            CATransaction.begin()
            CATransaction.setCompletionBlock(completion)
            box.bounds = CGRect(origin: .zero, size: target.size)
            box.add(animation, forKey: "bounds")
            CATransaction.commit()
        default:
            let driver = ControlDriver(window: window, from: origin, to: target, duration: seconds, done: completion)
            collapseControlKeepAlive.append(driver)
            driver.start()
        }
    }
    func cycle(_ left: Int) {
        if left == 0 { finish(); return }
        transition(to: small, from: big, seconds: collapseSeconds) {
            DispatchQueue.main.asyncAfter(deadline: .now() + Double(restMs) / 1000) {
                transition(to: big, from: small, seconds: expandSeconds) {
                    DispatchQueue.main.asyncAfter(deadline: .now() + Double(restMs) / 1000) { cycle(left - 1) }
                }
            }
        }
    }
    // Let the new window settle (and the recorder see it) before the first transition.
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { cycle(Int(cycles)) }
}

// MARK: - Burst capture (prototype verification only)
//
// While a transition runs, a background thread keeps capturing the window as the window server composited it, so the
// sequence shows what was really displayed frame after frame (jumps, stale content, material flashes), not what the
// driver intended. Frames are encoded on a second queue; `index.json` records when and where each one was taken.

/// Alpha (0–255) of one pixel of a captured window image, or nil for an unexpected pixel format.
private func collapseAlpha(_ image: CGImage, x: Int, y: Int) -> UInt8? {
    guard image.bitsPerPixel == 32, x >= 0, y >= 0, x < image.width, y < image.height,
          let data = image.dataProvider?.data, let bytes = CFDataGetBytePtr(data) else { return nil }
    let offset = y * image.bytesPerRow + x * 4
    let littleEndian = image.byteOrderInfo == .order32Little
    switch image.alphaInfo {
    case .premultipliedFirst, .first, .noneSkipFirst: return littleEndian ? bytes[offset + 3] : bytes[offset]
    case .premultipliedLast, .last, .noneSkipLast: return littleEndian ? bytes[offset] : bytes[offset + 3]
    default: return nil
    }
}

/// A frame is torn when the window's right or bottom edge is see-through: the content layer already has the new
/// (smaller) size while the window server still shows the old frame. Window corners are rounded, so only the edge
/// midpoints are probed.
private func collapseIsTorn(_ image: CGImage) -> Bool {
    let right = collapseAlpha(image, x: image.width - 3, y: image.height / 2) ?? 255
    let bottom = collapseAlpha(image, x: image.width / 2, y: image.height - 3) ?? 255
    return right < 128 || bottom < 128
}

private final class CollapseBurst {
    let windowNumber: UInt32
    let directory: String
    let maxFrames: Int
    /// Pause between captures in microseconds (0 = back to back).
    let pauseMicroseconds: UInt32
    /// Write every frame as a PNG; otherwise only torn frames are kept.
    let writeAll: Bool
    private(set) var tears = 0
    private let writer = DispatchQueue(label: "askhuman.collapse.burst.writer", qos: .utility)
    private let lock = NSLock()
    private var stopping = false
    private let finished = DispatchSemaphore(value: 0)
    private var entries: [[String: Any]] = []

    init(windowNumber: UInt32, directory: String, maxFrames: Int, pauseMicroseconds: UInt32, writeAll: Bool) {
        self.windowNumber = windowNumber
        self.directory = directory
        self.maxFrames = maxFrames
        self.pauseMicroseconds = pauseMicroseconds
        self.writeAll = writeAll
    }

    func start() {
        Thread.detachNewThread { [self] in
            run()
            finished.signal()
        }
    }

    private func shouldStop() -> Bool {
        lock.lock(); defer { lock.unlock() }
        return stopping || entries.count >= maxFrames
    }

    private func bounds() -> CGRect {
        guard let list = CGWindowListCopyWindowInfo([.optionIncludingWindow], CGWindowID(windowNumber)) as? [[String: Any]],
              let dictionary = list.first?[kCGWindowBounds as String] as? NSDictionary,
              let rect = CGRect(dictionaryRepresentation: dictionary as CFDictionary) else { return .zero }
        return rect
    }

    private func run() {
        guard let capture = collapseWindowListImage else { return }
        let origin = CACurrentMediaTime()
        while !shouldStop() {
            let began = CACurrentMediaTime()
            // kCGWindowListOptionIncludingWindow (1 << 3); BoundsIgnoreFraming (1 << 0) | NominalResolution (1 << 4)
            guard let image = capture(CGRect.null, 1 << 3, windowNumber, (1 << 0) | (1 << 4))?.takeRetainedValue() else {
                usleep(2000)
                continue
            }
            let ended = CACurrentMediaTime()
            let rect = bounds()
            let torn = collapseIsTorn(image)
            lock.lock()
            let index = entries.count
            if torn { tears += 1 }
            entries.append([
                "i": index, "t": (began - origin) * 1000, "cost": (ended - began) * 1000,
                "x": rect.minX, "y": rect.minY, "w": rect.width, "h": rect.height,
                "pw": image.width, "ph": image.height, "torn": torn,
            ])
            lock.unlock()
            let url = URL(fileURLWithPath: directory).appendingPathComponent(String(format: "frame-%04d.png", index))
            if pauseMicroseconds > 0 { usleep(pauseMicroseconds) }
            if !writeAll && !torn { continue }
            writer.async {
                guard let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else { return }
                CGImageDestinationAddImage(destination, image, nil)
                CGImageDestinationFinalize(destination)
            }
        }
    }

    /// Stops the capture thread, waits for the encoder and writes the index. Returns the number of frames.
    func stop() -> Int {
        lock.lock(); stopping = true; lock.unlock()
        finished.wait()
        writer.sync {}
        lock.lock(); let snapshot = entries; lock.unlock()
        if let data = try? JSONSerialization.data(withJSONObject: snapshot, options: [.prettyPrinted]) {
            try? data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("index.json"))
        }
        return snapshot.count
    }
}

private var collapseBurst: CollapseBurst?
private var collapseBurstTears = 0

/// Start capturing `window` into `directory` (created if needed). Returns 0 on success, 1 when capture is unavailable,
/// 2 when a burst is already running, 3 when the directory cannot be created.
@_cdecl("ah_collapse_burst_start")
func popupCollapseBurstStart(_ windowPointer: UnsafeMutableRawPointer, _ directory: UnsafePointer<CChar>,
                             _ maxFrames: Int32, _ pauseMicroseconds: Int32, _ writeAll: Int32) -> Int32 {
    let window = Unmanaged<NSWindow>.fromOpaque(windowPointer).takeUnretainedValue()
    guard collapseWindowListImage != nil else { return 1 }
    guard collapseBurst == nil else { return 2 }
    let path = String(cString: directory)
    do { try FileManager.default.createDirectory(atPath: path, withIntermediateDirectories: true) } catch { return 3 }
    let burst = CollapseBurst(windowNumber: UInt32(window.windowNumber), directory: path, maxFrames: Int(maxFrames),
                              pauseMicroseconds: UInt32(max(0, pauseMicroseconds)), writeAll: writeAll != 0)
    collapseBurst = burst
    burst.start()
    return 0
}

/// Stop the running burst; returns the number of frames written, or -1 when none was running.
@_cdecl("ah_collapse_burst_stop")
func popupCollapseBurstStop() -> Int32 {
    guard let burst = collapseBurst else { return -1 }
    collapseBurst = nil
    let frames = burst.stop()
    collapseBurstTears = burst.tears
    return Int32(frames)
}

/// Number of torn frames (see `collapseIsTorn`) in the burst that was stopped last.
@_cdecl("ah_collapse_burst_tears")
func popupCollapseBurstTears() -> Int32 {
    Int32(collapseBurstTears)
}
