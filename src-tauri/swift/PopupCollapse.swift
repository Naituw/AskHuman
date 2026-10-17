import AppKit
import QuartzCore

// Native driver that collapses / expands the inbox popup window (docs/specs/popup-inbox-collapse.md).
// It animates the NSWindow frame frame-by-frame on the main thread, locked to the display refresh,
// while the WKWebView keeps a fixed viewport (PopupCanvas.swift) so the page never reflows. Only public
// AppKit API is used. The single entry point is `ah_collapse_animate`; the screenshot / burst / forced-
// activation helpers used to verify it live in swift/prototype/ and are compiled only with the
// `collapse-prototype` Cargo feature, so they never reach the product binary.

// MARK: - cubic-bezier easing (CSS semantics)

/// Internal (not private): the prototype-only control animations reuse it.
struct CollapseBezier {
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

        // Safety net: the caller normally pins the canvas first (ah_popup_canvas_prepare).
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
