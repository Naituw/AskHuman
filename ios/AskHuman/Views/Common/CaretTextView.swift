import SwiftUI
import UIKit

/// An inline, growing editor that reveals the insertion point and optional related controls.
/// The native text view retains its selection and marked text across SwiftUI updates.
@MainActor
struct CaretTextView: UIViewRepresentable {
    let placeholder: String
    @Binding var text: String
    var focused: Binding<Bool>
    let lines: ClosedRange<Int>
    let identifier: String
    var revealTarget: InputRevealTarget?

    @Environment(\.isEnabled) private var enabled
    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(\.launchOptions) private var launch

    func makeUIView(context: Context) -> InlineTextView {
        let view = InlineTextView()
        view.delegate = context.coordinator
        configure(view)
        return view
    }

    func updateUIView(_ view: InlineTextView, context: Context) {
        context.coordinator.parent = self
        configure(view)
        // Reassigning text during composition cancels the input method's marked range.
        if view.text != text, view.markedTextRange == nil {
            view.text = text
            view.invalidateIntrinsicContentSize()
        }
        view.updatePlaceholder()
        context.coordinator.synchronizeFocus(in: view, enabled: enabled)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: InlineTextView, context: Context) -> CGSize? {
        guard let width = proposal.width, width > 0 else { return nil }
        return CGSize(width: width, height: uiView.height(for: width))
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    private func configure(_ view: InlineTextView) {
        _ = typeSize
        let font = UIFont.preferredFont(forTextStyle: .body, compatibleWith: view.traitCollection)
        if view.font != font { view.font = font }
        view.textColor = enabled ? .label : .secondaryLabel
        view.setEditable(enabled)
        view.lineRange = lines
        view.revealTarget = revealTarget
        view.placeholder.text = placeholder
        view.placeholder.font = view.font
        view.accessibilityIdentifier = identifier
        view.accessibilityLabel = placeholder
        #if DEBUG
        view.diagnostics = launch.mock && launch.inputCaretDiagnostics
        #endif
    }

    @MainActor
    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: CaretTextView
        private var focusUpdateQueued = false
        private var focusEnabled = true
        init(_ parent: CaretTextView) { self.parent = parent }

        func synchronizeFocus(in view: InlineTextView, enabled: Bool) {
            focusEnabled = enabled
            guard parent.focused.wrappedValue != view.isFirstResponder, !focusUpdateQueued else { return }
            focusUpdateQueued = true
            // Either responder transition can ask SwiftUI for focus values and reenter its view graph.
            // Apply it after this update, reading the latest binding so an old request cannot steal focus.
            DispatchQueue.main.async { [weak self, weak view] in
                guard let self else { return }
                focusUpdateQueued = false
                guard let view else { return }
                if parent.focused.wrappedValue {
                    if focusEnabled, view.isEditable, view.window != nil, !view.isFirstResponder {
                        view.becomeFirstResponder()
                    }
                } else if view.isFirstResponder {
                    view.resignFirstResponder()
                }
            }
        }

        func textViewDidBeginEditing(_ textView: UITextView) {
            if !parent.focused.wrappedValue { parent.focused.wrappedValue = true }
            (textView as? InlineTextView)?.requestReveal(resuming: true)
        }

        func textViewDidEndEditing(_ textView: UITextView) {
            if parent.focused.wrappedValue { parent.focused.wrappedValue = false }
            (textView as? InlineTextView)?.removeDiagnostics()
        }

        func textViewDidChange(_ textView: UITextView) {
            parent.text = textView.text
            textView.invalidateIntrinsicContentSize()
            (textView as? InlineTextView)?.updatePlaceholder()
            (textView as? InlineTextView)?.requestReveal(resuming: true)
        }

        func textViewDidChangeSelection(_ textView: UITextView) {
            (textView as? InlineTextView)?.requestReveal(resuming: true)
        }
    }
}

/// A question owns its editor and attachment entry point; no other question can supply a scroll target.
@MainActor
final class InputRevealTarget {
    enum Part { case editor, attachment }
    weak var editor: UIView?
    weak var attachment: UIView?
    weak var textView: InlineTextView?
    private var lastFrames: [CGRect?] = []

    func frame(_ part: Part, in view: UIView) -> CGRect? {
        let anchor = part == .editor ? editor : attachment
        guard let anchor, let window = anchor.window, window === (view as? UIWindow ?? view.window), !anchor.bounds.isEmpty else { return nil }
        return anchor.convert(anchor.bounds, to: view)
    }

    func geometryChanged() {
        guard let textView, let page = textView.ancestorScrolls.first else { return }
        // Content coordinates stay unchanged during a deliberate scroll through earlier text.
        let frames = [frame(.editor, in: page), frame(.attachment, in: page)]
        guard frames != lastFrames else { return }
        lastFrames = frames
        textView.requestReveal()
    }
}

struct InputRevealAnchor: UIViewRepresentable {
    let target: InputRevealTarget?
    let part: InputRevealTarget.Part

    func makeUIView(context: Context) -> Anchor { Anchor() }
    func updateUIView(_ view: Anchor, context: Context) { view.bind(target, part: part) }
    // Background markers need the proposed size; a plain UIView has no useful intrinsic size.
    func sizeThatFits(_ proposal: ProposedViewSize, uiView: Anchor, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? 0, height: proposal.height ?? 0)
    }

    final class Anchor: UIView {
        private var target: InputRevealTarget?
        private var part = InputRevealTarget.Part.editor

        init() { super.init(frame: .zero); isUserInteractionEnabled = false }
        required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

        func bind(_ target: InputRevealTarget?, part: InputRevealTarget.Part) {
            if self.target !== target || self.part != part { detach() }
            self.target = target
            self.part = part
            attach()
        }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            if window == nil { detach() } else { attach() }
        }

        override func layoutSubviews() { super.layoutSubviews(); target?.geometryChanged() }

        private func attach() {
            guard window != nil else { return }
            if part == .editor { target?.editor = self } else { target?.attachment = self }
            target?.geometryChanged()
        }

        private func detach() {
            if target?.editor === self { target?.editor = nil }
            if target?.attachment === self { target?.attachment = nil }
            target?.geometryChanged()
        }
    }
}

@MainActor
final class InlineTextView: UITextView {
    var lineRange = 1...10
    var revealTarget: InputRevealTarget? {
        didSet {
            guard oldValue !== revealTarget else { return }
            if oldValue?.textView === self { oldValue?.textView = nil }
            revealTarget?.textView = self
        }
    }
    let placeholder = UILabel()
    var diagnostics = false
    private var revealQueued = false
    private var lastSize = CGSize.zero
    private var lastContentSize = CGSize.zero
    private var keyboardFrame: CGRect?
    private var diagnosticLabel: UILabel?
    private var followsCaret = false
    /// Set while the keyboard is handed over from another input: the first scroll that brings this one into
    /// view slides, where a scroll after typing is a jump of a line.
    private var slidesIntoView = false
    private var observedPans: [UIPanGestureRecognizer] = []
    private let clearance: CGFloat = 12

    init() {
        super.init(frame: .zero, textContainer: nil)
        backgroundColor = .clear
        textContainerInset = .zero
        textContainer.lineFragmentPadding = 0
        contentInsetAdjustmentBehavior = .never
        adjustsFontForContentSizeCategory = true
        autocapitalizationType = .sentences
        setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        placeholder.textColor = .placeholderText
        placeholder.isUserInteractionEnabled = false
        placeholder.isAccessibilityElement = false
        addSubview(placeholder)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardChanged), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardChanged), name: UIResponder.keyboardDidChangeFrameNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardHidden), name: UIResponder.keyboardDidHideNotification, object: nil)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: height(for: max(1, bounds.width)))
    }

    func height(for width: CGFloat) -> CGFloat {
        let lineHeight = font?.lineHeight ?? 22
        let full = super.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
        return ceil(min(max(full, CGFloat(lineRange.lowerBound) * lineHeight), CGFloat(lineRange.upperBound) * lineHeight))
    }

    func updatePlaceholder() {
        placeholder.isHidden = !text.isEmpty
        setNeedsLayout()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        placeholder.frame = CGRect(origin: .zero, size: CGSize(width: bounds.width, height: font?.lineHeight ?? 22))
        if lastSize != bounds.size || lastContentSize != contentSize {
            lastSize = bounds.size
            lastContentSize = contentSize
            requestReveal()
        }
        publishDiagnostics()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        for pan in observedPans { pan.removeTarget(self, action: #selector(manualScroll)) }
        observedPans = window == nil ? [] : ([self] + ancestorScrolls).map(\.panGestureRecognizer)
        for pan in observedPans { pan.addTarget(self, action: #selector(manualScroll)) }
    }

    @objc private func manualScroll(_ pan: UIPanGestureRecognizer) {
        if pan.state == .began { followsCaret = false }
    }

    override func willMove(toWindow newWindow: UIWindow?) {
        if newWindow == nil { removeDiagnostics() }
        super.willMove(toWindow: newWindow)
    }

    private var wantsEditable = true
    private var editableChangeQueued = false

    /// Makes the text view editable or read-only (the form is disabled while an answer is on its way).
    ///
    /// A text view that has the keyboard resigns the moment it stops being editable, and a resign in the middle of a SwiftUI
    /// update pass runs the view graph again inside itself (the responder chain asks the hosting view whether it can take
    /// the responder over). With some content on the page that is an AttributeGraph cycle which kept the app from answering
    /// for a minute after "Send". So the switch to read-only waits until the pass is over; the other way round, and for a
    /// text view without the keyboard, nothing resigns and it is made at once.
    func setEditable(_ editable: Bool) {
        wantsEditable = editable
        guard isEditable != editable else { return }
        if editable || !isFirstResponder {
            isEditable = editable
            return
        }
        guard !editableChangeQueued else { return }
        editableChangeQueued = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            editableChangeQueued = false
            if isEditable != wantsEditable { isEditable = wantsEditable }
        }
    }

    /// Takes the keyboard over from the input that has it: UIKit moves the first responder in one step, so the
    /// keyboard neither leaves nor comes again, and the page slides to this input as the caret needs.
    ///
    /// This is done here, from a button's action, and not by SwiftUI through `updateUIView` (the old input
    /// resigning, this one becoming): a resign in the middle of an update pass runs the layout again inside
    /// itself, and the `becomeFirstResponder` of the other input that the pass makes in there is undone
    /// when the outer resign unwinds. The keyboard is left on screen with nothing that has it.
    @discardableResult
    func takeTheKeyboard() -> Bool {
        slidesIntoView = true
        let taken = becomeFirstResponder()
        if !taken { slidesIntoView = false }
        return taken
    }

    /// Coalesce input, selection and layout changes into one correction after layout settles.
    func requestReveal(resuming: Bool = false) {
        guard isFirstResponder else { return }
        if resuming { followsCaret = true }
        guard followsCaret, !revealQueued else { return }
        revealQueued = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.revealQueued = false
            self.revealCaret()
        }
    }

    @objc private func keyboardChanged(_ notification: Notification) {
        guard let window else { return }
        if let screen = notification.object as? UIScreen, screen != window.screen { return }
        keyboardFrame = (notification.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue
        requestReveal()
    }

    @objc private func keyboardHidden(_ notification: Notification) {
        keyboardFrame = nil
        requestReveal()
    }

    fileprivate var ancestorScrolls: [UIScrollView] {
        var result: [UIScrollView] = []
        var next = superview
        while let view = next {
            if let scroll = view as? UIScrollView { result.append(scroll) }
            next = view.superview
        }
        return result
    }

    /// Both the keyboard and the page's adjusted insets (including its action bar) obscure text.
    private func visibleRect(in window: UIWindow) -> CGRect {
        var visible = window.safeAreaLayoutGuide.layoutFrame
        for scroll in ancestorScrolls {
            visible = visible.intersection(scroll.convert(scroll.bounds.inset(by: scroll.adjustedContentInset), to: window))
        }
        if let root = window.rootViewController?.view {
            let guide = root.convert(root.keyboardLayoutGuide.layoutFrame, to: window)
            if guide.height > 0, guide.minY > visible.minY { visible.size.height = max(0, min(visible.maxY, guide.minY) - visible.minY) }
        }
        if let keyboardFrame {
            let keyboard = window.screen.coordinateSpace.convert(keyboardFrame, to: window).intersection(window.bounds)
            if !keyboard.isNull, keyboard.width > 0, keyboard.minY > visible.minY {
                visible.size.height = max(0, min(visible.maxY, keyboard.minY) - visible.minY)
            }
        }
        return visible
    }

    private func revealCaret() {
        guard followsCaret, isFirstResponder, let window, let selection = selectedTextRange else { return }
        let scrolls = ancestorScrolls
        // A deliberate gesture may move away from the caret; typing or a new selection follows it again.
        guard !isDragging, !isDecelerating, !scrolls.contains(where: { $0.isDragging || $0.isDecelerating }) else { return }
        let slides = slidesIntoView
        slidesIntoView = false
        window.layoutIfNeeded()
        layoutIfNeeded()
        let caret = caretRect(for: selection.end)
        let ownViewport = bounds.inset(by: adjustedContentInset)
        if caret.maxY > ownViewport.maxY { scroll(self, by: caret.maxY - ownViewport.maxY) }
        else if caret.minY < ownViewport.minY { scroll(self, by: caret.minY - ownViewport.minY) }
        for scroll in scrolls {
            let caret = convert(caretRect(for: selection.end), to: window)
            let visible = visibleRect(in: window).insetBy(dx: 0, dy: clearance)
            guard !visible.isNull, visible.height >= caret.height else { continue }
            var target = caret
            if let attachment = revealTarget?.frame(.attachment, in: window) {
                let combined = caret.union(attachment)
                if combined.height <= visible.height {
                    target = combined
                    if let editor = revealTarget?.frame(.editor, in: window), editor.union(combined).height <= visible.height {
                        target = editor.union(combined)
                    }
                }
            }
            let delta: CGFloat
            if target.maxY > visible.maxY { delta = target.maxY - visible.maxY }
            else if target.minY < visible.minY { delta = target.minY - visible.minY }
            else { continue }
            self.scroll(scroll, by: delta, sliding: slides)
        }
        publishDiagnostics()
    }

    private func scroll(_ view: UIScrollView, by delta: CGFloat, sliding: Bool = false) {
        let minimum = -view.adjustedContentInset.top
        let maximum = max(minimum, view.contentSize.height - view.bounds.height + view.adjustedContentInset.bottom)
        let y = min(maximum, max(minimum, view.contentOffset.y + delta))
        guard abs(y - view.contentOffset.y) > 0.5 else { return }
        let offset = CGPoint(x: view.contentOffset.x, y: y)
        if sliding, abs(delta) > 40, !UIAccessibility.isReduceMotionEnabled {
            // The offset is set inside the animation block, so it reads as its final value at once and the
            // corrections that follow (the keyboard says again where it is) find nothing left to do, instead
            // of cutting the slide short.
            UIView.animate(withDuration: 0.3, delay: 0, options: [.curveEaseInOut, .allowUserInteraction, .beginFromCurrentState]) {
                view.contentOffset = offset
            }
        } else {
            view.setContentOffset(offset, animated: false)
        }
    }

    /// Native UI tests inspect the measured caret, not a predicted scroll target. Only mock Debug runs expose it.
    private func publishDiagnostics() {
        #if DEBUG
        guard diagnostics, isFirstResponder, let window, let selection = selectedTextRange else { return }
        if diagnosticLabel == nil {
            let label = UILabel()
            label.text = "."
            label.textColor = .clear
            label.isUserInteractionEnabled = false
            label.isAccessibilityElement = true
            label.accessibilityLabel = "Input caret geometry"
            diagnosticLabel = label
            window.addSubview(label)
        }
        let caret = convert(caretRect(for: selection.end), to: window)
        let visible = visibleRect(in: window)
        let clip = visible.intersection(convert(bounds.inset(by: adjustedContentInset), to: window))
        let attachment = revealTarget?.frame(.attachment, in: window)
        diagnosticLabel?.frame = CGRect(x: 1, y: window.safeAreaInsets.top + 1, width: 1, height: 1)
        diagnosticLabel?.accessibilityIdentifier = (accessibilityIdentifier ?? "input") + "-caret-state"
        diagnosticLabel?.accessibilityValue = "top=\(caret.minY);bottom=\(caret.maxY);visibleTop=\(visible.minY);visibleBottom=\(visible.maxY);clipTop=\(clip.minY);clipBottom=\(clip.maxY);location=\(selectedRange.location);length=\(text.utf16.count);attachmentTop=\(attachment?.minY ?? -1);attachmentBottom=\(attachment?.maxY ?? -1);anchorHeight=\(revealTarget?.attachment?.bounds.height ?? -1)"
        #endif
    }

    func removeDiagnostics() {
        diagnosticLabel?.removeFromSuperview()
        diagnosticLabel = nil
    }
}
