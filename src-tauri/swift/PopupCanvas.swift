import AppKit
import WebKit
import QuartzCore

// The main origin never changes between responsive layout and a frozen pane transition.
// Normal edge drags use AppKit autoresizing; only pane animations reserve a fixed viewport.
final class PopupCanvas: NSView {
    override var isFlipped: Bool { true }
    weak var webview: NSView?
}
private var popupCanvas: PopupCanvas?
func popupPreviewParent(_ window: NSWindow) -> NSView? {
    if let canvas = popupCanvas, canvas.window === window { return canvas }
    return window.contentView
}
func popupCanvasIsResponsive(_ window: NSWindow) -> Bool {
    guard let canvas = popupCanvas, canvas.window === window else { return false }
    return canvas.autoresizingMask.contains(.width)
}

@_cdecl("ah_popup_canvas_prepare")
func preparePopupCanvas(_ pointer: UnsafeMutableRawPointer, _ left: Double,
                        _ width: Double, _ height: Double, _ visibleLeft: Double) {
    let webview = Unmanaged<NSView>.fromOpaque(pointer).takeUnretainedValue()
    let canvas: PopupCanvas
    if let current = popupCanvas, current.webview === webview {
        canvas = current
    } else {
        guard let parent = webview.superview else { return }
        canvas = PopupCanvas(frame: webview.frame)
        canvas.webview = webview
        canvas.autoresizingMask = []
        canvas.wantsLayer = true
        canvas.layer?.masksToBounds = true
        parent.addSubview(canvas, positioned: .above, relativeTo: webview)
        webview.removeFromSuperview()
        canvas.addSubview(webview)
        webview.autoresizingMask = []
        parent.wantsLayer = true
        parent.layer?.masksToBounds = true
        popupCanvas = canvas
    }
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    canvas.autoresizingMask = []
    webview.autoresizingMask = []
    popupPreviewResizing(false)
    let parent = canvas.superview!
    canvas.frame = NSRect(x: visibleLeft - left,
        y: parent.isFlipped ? 0 : parent.bounds.height - height, width: width, height: height)
    webview.frame = NSRect(x: 0, y: 0, width: width, height: height)
    CATransaction.commit()
}

@_cdecl("ah_popup_canvas_resume")
func resumePopupCanvas(_ pointer: UnsafeMutableRawPointer, _ left: Double,
                       _ visibleLeft: Double, _ minimumWidth: Double, _ minimumHeight: Double) {
    let webview = Unmanaged<NSView>.fromOpaque(pointer).takeUnretainedValue()
    guard let canvas = popupCanvas, canvas.webview === webview,
          let parent = canvas.superview, let window = canvas.window else { return }
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    // CSS is still pinned while the unused reserved viewport is removed. The following
    // responsive layout therefore has the same used widths and the same main origin.
    let width = parent.bounds.width + left - visibleLeft
    let height = parent.bounds.height
    canvas.frame = NSRect(x: visibleLeft - left, y: 0, width: width, height: height)
    webview.frame = NSRect(x: 0, y: 0, width: width, height: height)
    canvas.autoresizingMask = [.width, .height]
    webview.autoresizingMask = [.width, .height]
    popupPreviewResizing(true)
    window.minSize = NSSize(width: minimumWidth, height: minimumHeight)
    CATransaction.commit()
}

@_cdecl("ah_popup_canvas_position")
func positionPopupCanvas(_ pointer: UnsafeMutableRawPointer, _ x: Double, _ height: Double) {
    let webview = Unmanaged<NSView>.fromOpaque(pointer).takeUnretainedValue()
    guard let canvas = popupCanvas, canvas.webview === webview, let parent = canvas.superview else { return }
    // The viewport extent stays fixed for every frame of a pane transition.
    canvas.setFrameOrigin(NSPoint(x: x, y: parent.isFlipped ? 0 : parent.bounds.height - height))
}
