import SwiftUI
import UIKit
import AHCore

/// A zero-size attachment to the real scene, with a separate, non-key decorative window.
struct ArrivalSceneHost: UIViewRepresentable {
    var arrivals: [RequestArrival]
    var pendingCount: Int
    var reduceMotion: Bool
    var diagnostics = false
    var slowdown = 1.0
    var compositionFixture = false
    var finish: (String) -> Void

    func makeCoordinator() -> ArrivalSceneController { ArrivalSceneController() }

    func makeUIView(context: Context) -> ArrivalSceneAttachment {
        let view = ArrivalSceneAttachment()
        view.isUserInteractionEnabled = false
        view.attached = { [weak controller = context.coordinator] window in controller?.attach(to: window) }
        return view
    }

    func updateUIView(_ view: ArrivalSceneAttachment, context: Context) {
        context.coordinator.synchronize(arrivals, pendingCount: pendingCount, reduceMotion: reduceMotion,
                                       diagnostics: diagnostics, slowdown: slowdown, compositionFixture: compositionFixture, finish: finish)
    }

    static func dismantleUIView(_ view: ArrivalSceneAttachment, coordinator: ArrivalSceneController) {
        view.attached = nil
        coordinator.detach()
    }
}

final class ArrivalSceneAttachment: UIView {
    var attached: ((UIWindow?) -> Void)?
    override func didMoveToWindow() { super.didMoveToWindow(); attached?(window) }
}

/// Never takes first responder or participates in touch routing, even over a presented controller.
private final class ArrivalDecorationWindow: UIWindow {
    override var canBecomeKey: Bool { false }
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? { nil }
}

@MainActor
final class ArrivalSceneController: NSObject {
    private struct Run {
        var request: RequestArrival
        var began: CFTimeInterval
        var origin: CGPoint
        var target: CGPoint
        var offset: CGFloat
        var bubble: ArrivalBubbleView
    }

    private weak var source: UIWindow?
    private var decoration: ArrivalDecorationWindow?
    private var canvas: UIView?
    private let receiver = ArrivalReceiverView()
    private let glow = UIView()
    private let diagnostic = UILabel()
    private var displayLink: CADisplayLink?
    private var keyboard = CGRect.null
    private var runs: [String: Run] = [:]
    private var arrivals: [RequestArrival] = []
    private var finished = Set<String>()
    private var pendingCount = 0
    private var reduceMotion = false
    private var diagnostics = false
    private var slowdown = 1.0
    private var compositionFixture = false
    #if DEBUG
    private let compositionProbe = ArrivalCompositionProbe()
    #endif
    private var lastFrame = 0.0
    private var layoutAt = 0.0
    private var cachedLayout: ArrivalLayout?
    private var finish: ((String) -> Void)?
    private weak var shiftedTitle: UIView?
    private var titleTransform = CGAffineTransform.identity

    override init() {
        super.init()
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(keyboardChanged), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        center.addObserver(self, selector: #selector(keyboardHidden), name: UIResponder.keyboardWillHideNotification, object: nil)
        center.addObserver(self, selector: #selector(interrupted), name: UIApplication.willResignActiveNotification, object: nil)
        center.addObserver(self, selector: #selector(sceneInterrupted), name: UIScene.willDeactivateNotification, object: nil)
    }

    deinit { NotificationCenter.default.removeObserver(self) }

    func attach(to window: UIWindow?) {
        guard source !== window else { return }
        removeWindow()
        source = window
        guard let window, let scene = window.windowScene else { return }
        let overlay = ArrivalDecorationWindow(windowScene: scene)
        overlay.frame = window.frame
        overlay.windowLevel = UIWindow.Level(rawValue: window.windowLevel.rawValue + 1)
        overlay.backgroundColor = .clear
        overlay.isOpaque = false
        overlay.isUserInteractionEnabled = false
        overlay.accessibilityElementsHidden = !diagnostics
        let controller = UIViewController()
        let view = UIView(frame: window.bounds)
        view.isOpaque = false
        view.backgroundColor = .clear
        view.isUserInteractionEnabled = false
        controller.view = view
        overlay.rootViewController = controller
        decoration = overlay
        canvas = view
        receiver.alpha = 0
        view.addSubview(receiver)
        glow.isUserInteractionEnabled = false
        glow.backgroundColor = .clear
        glow.alpha = 0
        glow.layer.cornerRadius = 18
        let glowLayer = CAGradientLayer()
        glowLayer.type = .radial
        glowLayer.colors = [UIColor.systemBlue.cgColor, UIColor.clear.cgColor]
        glowLayer.startPoint = CGPoint(x: 0.5, y: 0.5)
        glowLayer.endPoint = CGPoint(x: 1, y: 1)
        glowLayer.frame = CGRect(x: 0, y: 0, width: 44, height: 44)
        glow.layer.addSublayer(glowLayer)
        view.addSubview(glow)
        diagnostic.isAccessibilityElement = true
        diagnostic.accessibilityIdentifier = "arrival-motion-state"
        diagnostic.accessibilityLabel = "Arrival motion"
        diagnostic.frame = CGRect(x: 1, y: window.safeAreaInsets.top + 1, width: 1, height: 1)
        diagnostic.accessibilityElementsHidden = !diagnostics
        view.addSubview(diagnostic)
        // Setting visibility is intentional: makeKeyAndVisible would dismiss the editing responder.
        startReadyRuns()
    }

    func synchronize(_ arrivals: [RequestArrival], pendingCount: Int, reduceMotion: Bool,
                     diagnostics: Bool, slowdown: Double, compositionFixture: Bool, finish: @escaping (String) -> Void) {
        self.arrivals = arrivals
        self.pendingCount = pendingCount
        self.reduceMotion = reduceMotion || UIAccessibility.isReduceMotionEnabled
        self.diagnostics = diagnostics
        self.slowdown = max(1, slowdown)
        self.compositionFixture = diagnostics && compositionFixture
        self.finish = finish
        let live = Set(arrivals.map(\.id))
        finished.formIntersection(live)
        for id in Array(runs.keys) where !live.contains(id) {
            runs.removeValue(forKey: id)?.bubble.removeFromSuperview()
        }
        decoration?.accessibilityElementsHidden = !diagnostics
        diagnostic.accessibilityElementsHidden = !diagnostics
        startReadyRuns()
        if runs.isEmpty { idle() }
    }

    func detach() {
        removeWindow()
        source = nil
        arrivals.removeAll()
        finish = nil
    }

    private func startReadyRuns() {
        guard let source, source.windowScene?.activationState == .foregroundActive, let canvas else { return }
        let layout = ArrivalLayout.measure(in: source, keyboard: keyboard)
        cachedLayout = layout
        layoutAt = CACurrentMediaTime()
        for request in arrivals where runs[request.id] == nil && !finished.contains(request.id) {
            let bubble = ArrivalBubbleView()
            bubble.isAccessibilityElement = diagnostics
            bubble.accessibilityIdentifier = "arrival-bubble-" + request.requestId
            bubble.accessibilityLabel = "Incoming request"
            canvas.addSubview(bubble)
            let offsets: [CGFloat] = [0, 53, -53, 26, -26]
            let offset = offsets[runs.count % offsets.count]
            let origin = CGPoint(x: min(source.bounds.maxX - 48, max(48, layout.origin.x + offset)), y: layout.origin.y)
            runs[request.id] = Run(request: request, began: CACurrentMediaTime(), origin: origin,
                                  target: layout.target, offset: offset, bubble: bubble)
        }
        guard !runs.isEmpty else { return }
        decoration?.isHidden = false
        if displayLink == nil {
            lastFrame = CACurrentMediaTime()
            let link = CADisplayLink(target: self, selector: #selector(tick))
            link.preferredFrameRateRange = CAFrameRateRange(minimum: 30, maximum: 120, preferred: 60)
            link.add(to: .main, forMode: .common)
            displayLink = link
        }
    }

    @objc private func tick(_ link: CADisplayLink) {
        guard let source, source.windowScene?.activationState == .foregroundActive else { interrupted(); return }
        decoration?.frame = source.frame
        canvas?.frame = source.bounds
        let now = CACurrentMediaTime(), dt = min(0.1, max(0, now - lastFrame))
        lastFrame = now
        if cachedLayout == nil || now - layoutAt >= 1.0 / 30 {
            cachedLayout = ArrivalLayout.measure(in: source, keyboard: keyboard)
            layoutAt = now
        }
        guard let layout = cachedLayout else { return }
        let ages = runs.values.map { (now - $0.began) * 1000 / slowdown }
        let receiverOpacity = layout.destination == .title
            ? (reduceMotion ? 1 : (ages.map { smooth(($0 - 1500) / 200) * (1 - smooth(($0 - ArrivalMotion.landedAt - 950) / 450)) }.max() ?? 0)) : 0
        receiver.update(count: pendingCount)
        receiver.center = layout.receiverCenter
        receiver.alpha = receiverOpacity
        shiftTitle(layout.title, opacity: receiverOpacity)
        var target = layout.target
        if layout.destination == .title, let canvas {
            let rect = receiver.tray.convert(receiver.tray.bounds, to: canvas)
            target = CGPoint(x: rect.midX, y: rect.midY)
        }
        var pulse = 0.0
        for id in Array(runs.keys) {
            guard var run = runs[id] else { continue }
            let elapsed = (now - run.began) * 1000 / slowdown
            let end = reduceMotion ? 850.0 : (layout.destination == .title
                ? ArrivalMotion.landedAt + 950 + 450 : ArrivalMotion.duration)
            if elapsed >= end {
                run.bubble.removeFromSuperview()
                runs[id] = nil
                finishRun(id)
                continue
            }
            // Follow moving keyboard/navigation anchors without teleporting an in-flight bubble.
            let blend = 1 - exp(-dt / 0.10)
            run.target = mix(run.target, target, blend)
            let desiredOrigin = CGPoint(x: min(source.bounds.maxX - 48, max(48, layout.origin.x + run.offset)), y: layout.origin.y)
            run.origin = mix(run.origin, desiredOrigin, blend)
            let state = ArrivalMotion.frame(at: elapsed, origin: .init(x: run.origin.x, y: run.origin.y),
                                            target: .init(x: run.target.x, y: run.target.y))
            let opacity = reduceMotion ? 0 : state.opacity * (1 - smooth((elapsed - ArrivalMotion.landedAt) / 160))
            run.bubble.paint(state, opacity: opacity)
            if reduceMotion { pulse = 0.12 }
            else if elapsed >= ArrivalMotion.landedAt {
                pulse = max(pulse, sin(.pi * min(1, (elapsed - ArrivalMotion.landedAt) / ArrivalMotion.land)) * 0.18)
            }
            runs[id] = run
        }
        glow.frame = CGRect(x: target.x - 22, y: target.y - 22, width: 44, height: 44)
        glow.alpha = layout.destination == .inbox ? pulse : 0
        if diagnostics {
            let age = ages.max() ?? 0
            let phase = reduceMotion ? "static" : (age < ArrivalMotion.flightAt ? "bounce" : (age < ArrivalMotion.landedAt ? "flight" : "landed"))
            diagnostic.accessibilityValue = "active=\(runs.count);destination=\(layout.destination == .inbox ? "inbox" : "title");key=\(source.isKeyWindow ? "source" : "other");touch=pass;phase=\(phase)"
            #if DEBUG
            if compositionFixture {
                diagnostic.accessibilityValue = (diagnostic.accessibilityValue ?? "") + ";composition=" + compositionProbe.inspect(in: source)
            }
            #endif
        }
        if runs.isEmpty { idle() }
    }

    private func finishRun(_ id: String) {
        finished.insert(id)
        let callback = finish
        Task { @MainActor in callback?(id) }
    }

    private func shiftTitle(_ title: UIView?, opacity: Double) {
        if shiftedTitle !== title {
            shiftedTitle?.transform = titleTransform
            shiftedTitle = title
            titleTransform = title?.transform ?? .identity
        }
        title?.transform = titleTransform.translatedBy(x: 0, y: -6 * opacity)
    }

    private func idle() {
        displayLink?.invalidate()
        displayLink = nil
        decoration?.isHidden = true
        shiftTitle(nil, opacity: 0)
        receiver.alpha = 0
        glow.alpha = 0
        cachedLayout = nil
    }

    private func removeWindow() {
        for id in Array(runs.keys) { finishRun(id) }
        idle()
        for run in runs.values { run.bubble.removeFromSuperview() }
        runs.removeAll()
        decoration?.rootViewController = nil
        decoration = nil
        canvas = nil
        cachedLayout = nil
    }

    @objc private func interrupted() {
        for id in Array(runs.keys) { finishRun(id) }
        for run in runs.values { run.bubble.removeFromSuperview() }
        runs.removeAll()
        idle()
    }

    @objc private func sceneInterrupted(_ note: Notification) {
        guard let scene = note.object as? UIWindowScene, scene === source?.windowScene else { return }
        interrupted()
    }

    @objc private func keyboardChanged(_ note: Notification) {
        guard let source, let frame = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect else { return }
        keyboard = source.convert(frame, from: nil)
        layoutAt = 0
    }

    @objc private func keyboardHidden(_ note: Notification) { keyboard = .null; layoutAt = 0 }

    private func smooth(_ n: Double) -> Double { let t = max(0, min(1, n)); return t * t * (3 - 2 * t) }
    private func mix(_ a: CGPoint, _ b: CGPoint, _ t: Double) -> CGPoint {
        CGPoint(x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t)
    }
}
