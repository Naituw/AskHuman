import SwiftUI
import UIKit
import AHCore
import AHProtocol
import AHStore

/// The long form for a question request: message, one card per question, a fixed bottom bar.
///
/// With several questions the bar offers "Next" until the last question has been on screen, and
/// only then "Send", as the Mac popup does: a stray tap cannot send answers that were never read.
/// "Next" itself follows where the screen is (`QuestionNavigator`), never what was looked at before,
/// so that a question that was seen but not answered is not jumped over. A person who is typing is
/// where the keyboard is, though: "Next" continues from that question (spec `ios-answer-flow.md` F4, F12, F13).
struct AnswerScreen: View {
    let item: InboxItem
    @Environment(AppStore.self) private var store
    @State private var tracker = ReadingTracker()
    /// The question "Next" last led to, which flashes once it has arrived.
    @State private var landing = Landing()
    /// The input of each question, to hand the keyboard from one to the next (`carryTheKeyboard`).
    @State private var inputs = InputTargets()
    /// The question whose text field has the keyboard.
    @State private var focused: Int?
    @State private var messageReady = false
    @Environment(\.launchOptions) private var launch

    fileprivate static let space = "answer-scroll"

    var body: some View {
        if let ask = item.view?.ask, let composer = store.composer(for: item) {
            let picker = store.todoPicker(for: item)
            // Set once the request has ended somewhere else while the page is open (§3.7): a strip says so, the
            // body fades and nothing takes an answer.
            let finished = store.endedNotice(for: item.id)
            let ended = finished != nil
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        if case .failed = item.state { FailedSubmissionNotice(item: item) }
                        if store.isEscalated(item), !ended { EscalationNotice(item: item) }
                        RequestHeader(item: item)
                        MessageCard(ask: ask, macId: item.macId, onSettled: { tracker.messageSettled(); messageReady = true })
                            .endedDimming(ended)
                        VStack(spacing: 0) {
                            ForEach(Array(ask.questions.enumerated()), id: \.offset) { index, question in
                                QuestionCard(
                                    index: index, count: ask.questions.count, ask: ask, question: question, composer: composer,
                                    landing: landing, focus: $focused, inputReveal: inputs.target(index), picker: picker
                                )
                                    .background(FrameReader(.card(index)))
                                    .background(RequestReadingAnchor(name: Self.cardID(index)))
                                    .disabled(composer.isSending || sending || ended)
                                    // The gap belongs to what "Next" scrolls to, so that a card does not end flush with the bar.
                                    .padding(.bottom, 16)
                                    .id(Self.cardID(index))
                            }
                            // What's next lists the queue among its options; every other ask gets a card for it.
                            if !ask.whatsNext, let picker, picker.isVisible {
                                TodoPickCard(picker: picker)
                                    .disabled(composer.isSending || sending || ended)
                                    .padding(.bottom, 16)
                            }
                        }
                        .endedDimming(ended)
                    }
                    .readableWidth()
                    .padding([.horizontal, .top])
                    .padding(.bottom, 8)
                    .background(RequestReadingAnchor(name: "request-top"))
                    .background(RequestScrollPosition(id: item.id, store: store, ready: messageReady || !MessageCard.drawnByWebPage(ask, launch: launch)))
                }
                .background(Color(.systemGroupedBackground))
                // A background stays between the bars, so its frame is the part of the screen the form is seen in.
                .background(FrameReader(.visible))
                .scrollDismissesKeyboard(.interactively)
                .actionBar {
                    SendBar(
                        item: item, composer: composer, moreBelow: tracker.moreBelow, ended: ended,
                        connection: store.state.requestConnection(macId: item.macId),
                        onSend: { send(composer) },
                        onQuickReply: { reply in send(composer, quickReply: reply) },
                        onNext: { next(composer, proxy: proxy) },
                        onJump: { jumpToMissing(composer, proxy: proxy) }
                    )
                }
                .endedStrip(finished)
                .endedCue(finished)
                .coordinateSpace(name: Self.space)
                .onPreferenceChange(ReadingKey.self) { tracker.update($0, composer: composer) }
                // The queue is read each time the page comes up.
                .task(id: item.id) { await picker?.load() }
                // The Mac refused the answer because a picked todo is gone: the queue is read again (and what
                // is gone is let go of), and the notice says what to do.
                .onChange(of: item.state) { _, state in
                    if case .failed(_, Wire.todoGone) = state { picker?.reload() }
                }
                .task(id: item.id) {
                    await tracker.begin(
                        composer, awaitingMessage: MessageCard.drawnByWebPage(ask, launch: launch),
                        // The page's own watchdog gives up on it after this long (and the native renderer settles).
                        patience: .seconds(launch.webTimeout + 1)
                    )
                }
            }

        }
    }

    fileprivate static func cardID(_ index: Int) -> String { "question-\(index)" }

    private var sending: Bool { if case .sending = item.state { true } else { false } }

    private func send(_ composer: AnswerComposer, quickReply: String? = nil) {
        Task {
            do {
                try await store.send(composer, item: item, quickReply: quickReply)
            } catch {
                store.reportRequestError(error.userMessage, for: item.id)
            }
        }
    }

    /// "Next". A person who is typing is in the question that has the keyboard, so it leads to the question
    /// after that one, whatever the screen shows (F12); when that is a plain-text question the keyboard moves
    /// to it and stays up (F13). Otherwise it leads to the first question whose bottom is not on screen yet:
    /// all of it when it fits (as the popup's "next" does), else its top, and on the next press its bottom.
    /// Nothing that was seen or answered before matters, only where the screen is now.
    private func next(_ composer: AnswerComposer, proxy: ScrollViewProxy) {
        // A second press while the keyboard is still being taken away would work out the same step twice.
        guard !tracker.advancing else { return }
        tracker.advancing = true
        let typing = focused
        Task {
            defer { tracker.advancing = false }
            if let typing, typing + 1 < composer.questionCount, composer.isTextOnly(typing + 1),
               carryTheKeyboard(to: typing + 1) {
                return
            }
            await putTheKeyboardAway()
            guard let viewport = tracker.viewport() else { return }
            let target = typing.flatMap { QuestionNavigator.next(after: $0, in: viewport, count: composer.questionCount) }
                ?? QuestionNavigator.next(in: viewport, count: composer.questionCount)
            guard let target else { return }
            go(to: target, proxy: proxy)
        }
    }

    /// Moves the keyboard to the input of the question that comes next and keeps it up: the page slides to
    /// the caret as it follows it when a field is tapped, and the card flashes like any other landing.
    /// `false` when the input cannot take it, and the keyboard is where it was.
    ///
    /// UIKit hands the keyboard over (`InlineTextView.takeTheKeyboard`) rather than this setting `focused`
    /// for SwiftUI to make the two inputs resign and become in its update pass: the nested layout of a resign
    /// in there undoes the other input's `becomeFirstResponder`, and the keyboard stays with nothing typing into it.
    private func carryTheKeyboard(to question: Int) -> Bool {
        guard inputs.target(question).textView?.takeTheKeyboard() == true else { return false }
        landing.flash(question)
        return true
    }

    /// Leads to the first question of a select-only form that still has no selection (what "Send"
    /// is waiting for).
    private func jumpToMissing(_ composer: AnswerComposer, proxy: ScrollViewProxy) {
        Task {
            await putTheKeyboardAway()
            guard let question = composer.firstWithoutSelection else { return }
            if let viewport = tracker.viewport(), let target = QuestionNavigator.reveal(question, in: viewport) {
                go(to: target, proxy: proxy)
            } else {
                landing.flash(question)
            }
        }
    }

    private func go(to target: QuestionNavigator.Target, proxy: ScrollViewProxy) {
        // A question that is all on screen is only pointed at.
        if target.anchor != .stay {
            withAnimation(.easeInOut(duration: 0.4)) {
                proxy.scrollTo(Self.cardID(target.question), anchor: target.anchor == .bottom ? .bottom : .top)
            }
        }
        landing.flash(target.question)
    }

    /// The keyboard covers the question that is being led to, and its going changes the room there is: let
    /// it go first, and the scroll is worked out for the screen that is left.
    ///
    /// It is resigned here, from the action of a button, and not by clearing `focused` and leaving it to
    /// `CaretTextView.updateUIView`: a keyboard that goes in the middle of a SwiftUI update leaves the bar and
    /// the scroll area where the keyboard had put them (a blank band under them) until something else makes the
    /// screen lay out again, and the card typed in, which reaches a point or two under the bar then, is taken
    /// for one not reached. Nor is a fixed wait any good: the layout takes the room back in a few frames, the
    /// keyboard itself needs about 0.4 s. So this waits for the layout, not for the clock, and the page is
    /// worked out (and scrolls) while the keyboard is still on its way down.
    private func putTheKeyboardAway() async {
        guard focused != nil else { return }
        let before = tracker.viewport()?.bottom
        let keyboard = KeyboardExit()
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        focused = nil
        // No word of a keyboard going (a hardware keyboard has none on screen): nothing to wait for.
        guard await keyboard.waitForStart(within: .milliseconds(150)) else { return }
        await tracker.settle(from: before, timeout: .milliseconds(500))
    }
}

/// The input of every question, kept apart from the cards so that the screen can reach it.
@MainActor
private final class InputTargets {
    private var targets: [Int: InputRevealTarget] = [:]

    func target(_ question: Int) -> InputRevealTarget {
        if let known = targets[question] { return known }
        let target = InputRevealTarget()
        targets[question] = target
        return target
    }
}

/// Notes that a keyboard has started to leave: the notification is what says there is something to wait for.
/// (An observer of this kind goes away with the object, so nothing is removed by hand.)
@MainActor
private final class KeyboardExit {
    private(set) var started = false

    init() {
        NotificationCenter.default.addObserver(
            self, selector: #selector(keyboardWillHide), name: UIResponder.keyboardWillHideNotification, object: nil
        )
    }

    @objc private func keyboardWillHide(_ notification: Notification) { started = true }

    /// Whether the keyboard started to leave within `timeout`.
    func waitForStart(within timeout: Duration) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while !started, ContinuousClock.now < deadline, !Task.isCancelled {
            try? await Task.sleep(for: .milliseconds(10))
        }
        return started
    }
}

/// The question the last "Next" led to. The card flashes when it is told again (`token` counts).
struct Landing: Equatable {
    private(set) var question: Int?
    private(set) var token = 0

    mutating func flash(_ question: Int) {
        self.question = question
        token += 1
    }
}

// MARK: reading progress

/// What the form measured of its own layout, in the coordinate space of the scrolling screen.
private struct Reading: Equatable {
    /// The frame of every question card.
    var cards: [Int: CGRect] = [:]
    /// The part of the scrolling screen that can be seen: from under the navigation bar down to the top of
    /// the bottom bar, which the keyboard moves up when it is there.
    var visible: CGRect?
}

private struct ReadingKey: PreferenceKey {
    static var defaultValue = Reading()

    static func reduce(value: inout Reading, nextValue: () -> Reading) {
        let next = nextValue()
        value.cards.merge(next.cards) { $1 }
        if let visible = next.visible { value.visible = visible }
    }
}

private struct FrameReader: View {
    enum Part {
        case card(Int)
        case visible
    }

    let part: Part

    init(_ part: Part) { self.part = part }

    var body: some View {
        GeometryReader { proxy in
            Color.clear.preference(key: ReadingKey.self, value: reading(proxy))
        }
    }

    private func reading(_ proxy: GeometryProxy) -> Reading {
        switch part {
        case .card(let index):
            return Reading(cards: [index: proxy.frame(in: .named(AnswerScreen.space))])
        case .visible:
            return Reading(visible: proxy.frame(in: .named(AnswerScreen.space)))
        }
    }
}

/// Marks the questions that were on screen, and knows where the screen is. Almost all of it is kept
/// out of observation on purpose: its numbers change on every frame of a scroll and must not make the
/// screen render again. Only what the bar shows is observed, and it is only written when it changes.
@MainActor
@Observable
private final class ReadingTracker {
    /// Whether a question is left below the screen, which is when the bar offers "Next" beside "Send".
    private(set) var moreBelow = false

    @ObservationIgnored private var latest = Reading()
    @ObservationIgnored private var live = false
    @ObservationIgnored private var messageIsLaidOut = false
    @ObservationIgnored private var questionCount = 0

    /// A "Next" is under way and has not led anywhere yet, so that a second press does not start another.
    @ObservationIgnored var advancing = false

    /// The first layout passes are not trusted (the bars may not have their final size yet): the
    /// questions on screen are counted a moment after the form appeared. A message drawn by a web page
    /// has no height until the page reports it, and the cards below it would look like they are on screen
    /// while it is one line tall: nothing is counted before it was laid out (but a page that never says
    /// anything is not waited for for ever).
    func begin(_ composer: AnswerComposer, awaitingMessage: Bool, patience: Duration = .seconds(3)) async {
        live = false
        messageIsLaidOut = !awaitingMessage
        try? await Task.sleep(for: .milliseconds(150))
        let deadline = ContinuousClock.now + patience
        while !messageIsLaidOut, ContinuousClock.now < deadline, !Task.isCancelled {
            try? await Task.sleep(for: .milliseconds(50))
        }
        if awaitingMessage {
            // The new height reaches the cards' frames a frame or two later.
            try? await Task.sleep(for: .milliseconds(150))
        }
        guard !Task.isCancelled else { return }
        live = true
        markSeen(composer)
    }

    /// The message's page reported its first layout (or the native renderer took over).
    func messageSettled() { messageIsLaidOut = true }

    func update(_ reading: Reading, composer: AnswerComposer) {
        latest = reading
        questionCount = composer.questionCount
        if live { markSeen(composer) }
        let more = viewport().flatMap { QuestionNavigator.next(in: $0, count: questionCount) } != nil
        if more != moreBelow { moreBelow = more }
    }

    /// What the screen shows of the questions right now; `nil` before anything was measured.
    func viewport() -> QuestionNavigator.Viewport? {
        guard let visible = latest.visible else { return nil }
        return QuestionNavigator.Viewport(cards: latest.cards, top: visible.minY, bottom: visible.maxY)
    }

    /// Waits until the visible part of the screen has grown from `bottom` (the keyboard went and the bar
    /// came down) and stopped changing, or until `timeout`: a screen whose layout is not final is no
    /// ground to work out a scroll on. A few frames, in practice.
    func settle(from bottom: CGFloat?, timeout: Duration) async {
        let deadline = ContinuousClock.now + timeout
        var previous: CGRect?
        var steady = 0
        while ContinuousClock.now < deadline, !Task.isCancelled {
            try? await Task.sleep(for: .milliseconds(16))
            guard let visible = latest.visible else { continue }
            let grown = bottom.map { visible.maxY > $0 + 1 } ?? true
            steady = grown && visible == previous ? steady + 1 : 0
            previous = visible
            if steady >= 2 { return }
        }
    }

    /// A question counts as seen once its bottom is above the bottom bar, whether it is still on
    /// screen or already scrolled past (spec `multi-question-vertical.md` V1).
    private func markSeen(_ composer: AnswerComposer) {
        guard let visible = latest.visible else { return }
        for (index, frame) in latest.cards where frame.maxY <= visible.maxY + 1 {
            composer.markSeen(index)
        }
    }
}

// MARK: message

private struct MessageCard: View {
    let ask: AskView
    let macId: String
    /// The message has been laid out (see `WebMarkdownView`); said at once when it needs no page.
    var onSettled: () -> Void = {}

    /// What the card shows of the message: its text, trimmed.
    private static func text(of ask: AskView) -> String {
        ask.message.text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Whether a web page draws the message, whose height is then known only once the page said so.
    static func drawnByWebPage(_ ask: AskView, launch: LaunchOptions) -> Bool {
        launch.webMarkdown && ask.isMarkdown && !text(of: ask).isEmpty
    }

    var body: some View {
        let text = Self.text(of: ask)
        if !text.isEmpty || !ask.message.files.isEmpty {
            VStack(alignment: .leading, spacing: 12) {
                if !text.isEmpty {
                    if ask.isMarkdown {
                        WebMarkdownView(text: text, onSettled: onSettled)
                    } else {
                        Text(verbatim: text).font(.body).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                if !ask.message.files.isEmpty {
                    AssetList(macId: macId, assets: ask.message.files)
                }
            }
            .card()
        }
    }
}

// MARK: questions

private struct QuestionCard: View {
    let index: Int
    let count: Int
    let ask: AskView
    let question: QuestionView
    let composer: AnswerComposer
    let landing: Landing
    var focus: Binding<Int?>
    let inputReveal: InputRevealTarget
    /// What's next lists the project's whole queue through it (D29); other asks have a card of their own.
    var picker: RequestTodoPicker?

    /// The picker that lists the queue in place of the todos the question carries, when this is What's next
    /// and the Mac can give the queue.
    private var queue: RequestTodoPicker? {
        guard ask.whatsNext, let picker, picker.listsQueue else { return nil }
        return picker
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                if count > 1 {
                    Text("\(index + 1)")
                        .font(.footnote.weight(.bold))
                        .foregroundStyle(.white)
                        .frame(width: 22, height: 22)
                        .background(composer.isAnswered(index) ? Color.green : Color.accentColor, in: Circle())
                        .accessibilityLabel("Question \(index + 1) of \(count)")
                }
                Group {
                    if ask.isMarkdown {
                        MarkdownView(text: question.text, baseFont: .body.weight(.semibold))
                    } else {
                        Text(verbatim: question.text).font(.body.weight(.semibold))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if !question.options.isEmpty {
                let listing = self.queue
                // The queue stands where the question's own todos are (just before "end" when it has none).
                let slot = listing == nil ? nil : (question.options.firstIndex { $0.todo } ?? question.options.count - 1)
                VStack(spacing: 8) {
                    ForEach(Array(question.options.enumerated()), id: \.offset) { optionIndex, option in
                        if let listing, optionIndex == slot, listing.isVisible {
                            TodoQueueRows(picker: listing)
                        }
                        if listing == nil || !option.todo {
                            OptionRow(
                                option: option, single: composer.single,
                                selected: composer.isSelected(question: index, option: optionIndex)
                            ) {
                                composer.toggle(question: index, option: optionIndex)
                            }
                            .accessibilityIdentifier("option-\(index)-\(optionIndex)")
                        }
                    }
                }
            }
            if !composer.selectOnly {
                CaretTextView(
                    placeholder: question.options.isEmpty ? String(localized: "Your answer") : String(localized: "Add a note (optional)"),
                    text: Binding(get: { composer.answers[index].text }, set: { composer.setText($0, question: index) }),
                    focused: Binding(get: { focus.wrappedValue == index }, set: { active in
                        if active { focus.wrappedValue = index }
                        else if focus.wrappedValue == index { focus.wrappedValue = nil }
                    }),
                    lines: 1...10, identifier: "answer-text-\(index)", revealTarget: inputReveal
                )
                .contentShape(Rectangle())
                .simultaneousGesture(TapGesture().onEnded { focus.wrappedValue = index })
                .padding(10)
                .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .background(InputRevealAnchor(target: inputReveal, part: .editor))
                ReplyAttachmentBar(composer: composer, question: index, revealTarget: inputReveal)
            }
        }
        .card()
        .overlay { LandingFlash(token: landing.question == index ? landing.token : nil) }
    }
}

/// The flash of a card that "Next" has led to (the popup's `flashCard`): an accent wash that fades.
private struct LandingFlash: View {
    /// Changes every time the card is to flash; `nil` for a card that is not the one led to.
    let token: Int?
    @State private var strength = 0.0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        RoundedRectangle(cornerRadius: 16, style: .continuous)
            .fill(Color.accentColor.opacity(0.14))
            .opacity(strength)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
            .task(id: token) {
                guard token != nil, !reduceMotion else { return }
                // Lights up as the scroll arrives, then fades.
                try? await Task.sleep(for: .milliseconds(180))
                guard !Task.isCancelled else { return }
                withTransaction(Transaction(animation: nil)) { strength = 1 }
                try? await Task.sleep(for: .milliseconds(40))
                guard !Task.isCancelled else { return }
                withAnimation(.easeOut(duration: 0.9)) { strength = 0 }
            }
    }
}

// MARK: options

/// One option of a question: a target the width of the card that turns on or off in a single frame.
/// Choosing should feel firm. The press shows at once (darker, a touch smaller, nothing fades), the
/// new state replaces the old one with no cross-fade, and a tick of haptic confirms it. (It used to be
/// a plain button whose choice faded in over about a third of a second, which felt soft.)
struct OptionRow: View {
    let text: String
    let recommended: Bool
    /// How many attachments the row carries (a todo's), shown as a chip.
    let attachments: Int
    let single: Bool
    let selected: Bool
    let action: () -> Void

    init(option: OptionView, single: Bool, selected: Bool, action: @escaping () -> Void) {
        text = option.text
        recommended = option.recommended
        attachments = 0
        self.single = single
        self.selected = selected
        self.action = action
    }

    /// A todo of the project's queue, listed like an option (docs/specs/special-replies.md D28/D29).
    init(todo: TodoItem, single: Bool, selected: Bool, action: @escaping () -> Void) {
        text = todo.text
        recommended = false
        attachments = Int(todo.att)
        self.single = single
        self.selected = selected
        self.action = action
    }

    /// Counts the presses, so that a tick is felt once per press: a single-choice form changes two
    /// rows when the choice moves, and each of them reacting to its own change would tick twice.
    @State private var presses = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Button {
            presses += 1
            action()
        } label: {
            HStack(alignment: .top, spacing: 12) {
                OptionMark(single: single, selected: selected, pops: !reduceMotion)
                Text(verbatim: text)
                    .font(.body)
                    .foregroundStyle(.primary)
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if recommended {
                    Chip(text: "Recommended", systemImage: "star.fill", tint: .orange)
                }
                if attachments > 0 {
                    Chip(verbatim: "\(attachments)", systemImage: "paperclip")
                        .accessibilityLabel("Attachments: \(attachments)")
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .frame(minHeight: 44)
            .contentShape(Rectangle())
        }
        .buttonStyle(OptionRowStyle(selected: selected))
        .sensoryFeedback(.selection, trigger: presses)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// The look of an option row, chosen or not, pressed or not.
private struct OptionRowStyle: ButtonStyle {
    let selected: Bool

    private let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)

    func makeBody(configuration: Configuration) -> some View {
        let pressed = configuration.isPressed
        configuration.label
            .background(shape.fill(fill(pressed: pressed)))
            .overlay(shape.strokeBorder(selected ? Color.accentColor : Color.clear, lineWidth: 2))
            .scaleEffect(pressed ? 0.985 : 1)
            // Only the press is animated, and quickly. The outer `transaction` clears whatever animation
            // an update brings along, so that choosing (or clearing) is one frame; this `animation`
            // sits inside it and sets the press's own.
            .animation(.easeOut(duration: 0.08), value: pressed)
            .transaction { $0.animation = nil }
    }

    private func fill(pressed: Bool) -> Color {
        if selected { return Color.accentColor.opacity(pressed ? 0.24 : 0.14) }
        return Color(pressed ? .secondarySystemFill : .tertiarySystemFill)
    }
}

/// The radio circle or the check box in front of an option. Drawn, not an SF Symbol swapped for
/// another, so that nothing cross-fades; the filled part pops in with a short spring.
private struct OptionMark: View {
    let single: Bool
    let selected: Bool
    let pops: Bool

    @ScaledMetric(relativeTo: .body) private var size: CGFloat = 22

    var body: some View {
        ZStack {
            if single {
                Circle()
                    .strokeBorder(selected ? Color.accentColor : Color(.tertiaryLabel), lineWidth: 1.5)
                Circle()
                    .fill(Color.accentColor)
                    .padding(size * 0.25)
                    .modifier(Pop(on: selected, animated: pops))
            } else {
                RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
                    .strokeBorder(selected ? Color.accentColor : Color(.tertiaryLabel), lineWidth: 1.5)
                RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
                    .fill(Color.accentColor)
                    .modifier(Pop(on: selected, animated: pops))
                Image(systemName: "checkmark")
                    .font(.system(size: size * 0.52, weight: .heavy))
                    .foregroundStyle(.white)
                    .modifier(Pop(on: selected, animated: pops))
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// Shows its content the moment `on` turns true (growing to size with a short spring) and hides it the
/// moment it turns false. The opacity is not animated.
private struct Pop: ViewModifier {
    let on: Bool
    let animated: Bool

    func body(content: Content) -> some View {
        content
            .scaleEffect(on ? 1 : 0.5)
            .animation(animated ? .spring(response: 0.22, dampingFraction: 0.6) : nil, value: on)
            .opacity(on ? 1 : 0)
    }
}

// MARK: send bar

private struct SendBar: View {
    let item: InboxItem
    let composer: AnswerComposer
    /// A question is left below the screen.
    let moreBelow: Bool
    let ended: Bool
    let connection: RequestConnection
    let onSend: () -> Void
    /// A quick reply of the bar menu: sends the form with it.
    let onQuickReply: (String) -> Void
    let onNext: () -> Void
    let onJump: () -> Void

    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(AppStore.self) private var store

    /// At the accessibility text sizes the buttons go under the text: side by side they would leave
    /// the words a few letters of width.
    private var stacked: Bool { typeSize.isAccessibilitySize }

    var body: some View {
        VStack(spacing: 8) {
            // The floating bar shows the upload inside the count chip, with the text it belongs to.
            if !BottomBar.floats, let progress = composer.uploadProgress {
                ProgressView(value: progress).progressViewStyle(.linear)
            }
            GlassGroup(spacing: 8) {
                VStack(alignment: .leading, spacing: 10) {
                    // On the floating bar the hint is a chip of its own above the row. In the row it would
                    // make the count chip two or three lines tall, and the buttons beside it a different
                    // height from it; this way the row keeps one height whatever the hint says.
                    if BottomBar.floats, let hint { hintView(hint, chip: true) }
                    // What the user armed from the menu: it goes with the answer, and the cross takes it back. Not
                    // there at all while nothing is armed: an empty view would still add the stack's spacing.
                    if composer.directive != nil { DirectivePanel(composer: composer) }
                    controls
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .classicBarBacking()
        .animation(.snappy(duration: 0.25), value: composer.hasReachedEnd)
        .animation(.snappy(duration: 0.25), value: moreBelow)
        .animation(.snappy(duration: 0.25), value: composer.directive?.kind)
        // Once the question is over (sent, or answered elsewhere) there is nothing left to arm: the panel goes, and with
        // it the keyboard of its note.
        .onChange(of: ended) { _, nowEnded in
            if nowEnded { composer.directive = nil }
        }
    }

    /// The count and the buttons.
    ///
    /// On the floating bar `CountAndButtons` lays them out: side by side while there is room for both, at the height of the
    /// tallest of them with each taking all of it (so that the chip and the capsules beside it are one height), and the
    /// count above the buttons when there is not. The accessibility text sizes always stack them. The classic bar is a plain
    /// row, whose buttons keep their own height whatever the count does.
    @ViewBuilder
    private var controls: some View {
        if BottomBar.floats, !stacked {
            CountAndButtons() {
                adaptiveCountChip
                buttons
            }
        } else {
            let layout = stacked ? AnyLayout(VStackLayout(alignment: .leading, spacing: 10)) : AnyLayout(HStackLayout(spacing: 12))
            layout {
                statusBlock
                if !stacked { Spacer(minLength: 8) }
                buttons
                    // "Send and go away" is the longest thing the row ever says. Left to the stack's even split, the count gets
                    // room it does not need and the words are cut ("Send and go…") on the classic bar; with the priority the
                    // buttons are sized first and only the count gives way.
                    .layoutPriority(composer.directive == nil ? 0 : 1)
            }
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// The row of buttons: the quick-reply menu, "Next", "Send". On the floating bar `ButtonRow` makes the two small ones
    /// circles as tall as "Send" (and so as wide), whatever the text size makes that.
    @ViewBuilder
    private var buttons: some View {
        if BottomBar.floats {
            ButtonRow { buttonItems }
        } else {
            HStack(spacing: 12) { buttonItems }
        }
    }

    /// The buttons themselves, the main one last (`ButtonRow` relies on that).
    @ViewBuilder
    private var buttonItems: some View {
        // Sending waits until the last question was on screen; until then the button leads
        // there. After that "Send" takes the main place, and "Next" stays beside it while there
        // are questions below the screen, as the popup's "Next" does until it reaches the last.
        if store.offersDirectives(for: item) {
            DirectiveMenu(composer: composer, disabled: ended || busy, canSend: enabled, onQuickReply: onQuickReply,
                          fillsRow: BottomBar.floats)
        }
        if composer.hasReachedEnd, moreBelow {
            nextIconButton.transition(.opacity.combined(with: .scale(scale: 0.92)))
        }
        primaryButton
    }

    /// Set on what sits in the row beside the buttons, to be as tall as the row.
    private var rowFill: CGFloat? { BottomBar.floats && !stacked ? .infinity : nil }

    @ViewBuilder
    private var statusBlock: some View {
        if BottomBar.floats {
            countChip
        } else {
            VStack(alignment: .leading, spacing: 2) {
                count
                if let hint { hintView(hint, chip: false) }
            }
        }
    }

    /// The count of the classic bar and of the accessibility text sizes. The row of the floating bar has
    /// `adaptiveCountChip`, which finds the wording that fits for itself.
    private var count: some View {
        // On the floating bar, an armed instruction makes the button say more ("Send and go away"), and the chip gives
        // up its word to make room for it.
        let compact = BottomBar.floats && composer.directive != nil
        return Group {
            if compact {
                Text(verbatim: "\(composer.answeredCount)/\(composer.questionCount)")
            } else {
                Text("Answered \(composer.answeredCount)/\(composer.questionCount)")
            }
        }
        .font(.subheadline.weight(.medium))
        // Only while an instruction is armed does the count have to give way to the longer button.
        .lineLimit(composer.directive == nil ? nil : 1)
        .minimumScaleFactor(composer.directive == nil ? 1 : 0.8)
        .accessibilityLabel(Text("Answered \(composer.answeredCount)/\(composer.questionCount)"))
        .accessibilityIdentifier("answered-count")
    }

    private var countChip: some View { chip(count) }

    /// The count chip of the side-by-side row (`CountAndButtons`). It says "Answered 1/3" while that fits beside the buttons,
    /// and "1/3" when only that does. Each wording keeps its own size however little room it is offered (`fixedSize`): that
    /// is how the layout can tell that even the short one does not fit, and puts the chip above the buttons.
    private var adaptiveCountChip: some View {
        chip(
            ViewThatFits(in: .horizontal) {
                countText(brief: false)
                countText(brief: true)
            }
        )
    }

    private func countText(brief: Bool) -> some View {
        Group {
            if brief {
                Text(verbatim: "\(composer.answeredCount)/\(composer.questionCount)")
            } else {
                Text("Answered \(composer.answeredCount)/\(composer.questionCount)")
            }
        }
        .font(.subheadline.weight(.medium))
        .lineLimit(1)
        .fixedSize()
        // VoiceOver says the whole of it, whichever wording is on screen.
        .accessibilityLabel(Text("Answered \(composer.answeredCount)/\(composer.questionCount)"))
        .accessibilityIdentifier("answered-count")
    }

    private func chip(_ label: some View) -> some View {
        let uploading = composer.uploadProgress
        return label
            // The progress line is drawn over the bottom of the text, in room kept for it, so that it takes
            // the width of the text and does not widen the chip.
            .padding(.bottom, uploading == nil ? 0 : 10)
            .overlay(alignment: .bottom) {
                if let uploading { ProgressView(value: uploading).progressViewStyle(.linear) }
            }
            .frame(maxWidth: stacked ? .infinity : nil, alignment: .leading)
            .barStatusChip(fillsHeight: !stacked)
    }

    /// "Next" until the last question was reached, then "Send" in the same place. It is one button, so
    /// that on the glass bar the capsule stays and only the words change, rather than one capsule
    /// going and another coming.
    private var primaryButton: some View {
        let reached = composer.hasReachedEnd
        let label: LocalizedStringKey = reached ? (busy ? "Sending…" : sendTitle) : "Next question"
        return Button(action: reached ? onSend : onNext) {
            ZStack {
                if reached {
                    HStack(spacing: 6) {
                        if busy { ProgressView().controlSize(.small).tint(.white) }
                        // "Send and go away" is longer than "Send": it shrinks a little before it would be cut.
                        Text(busy ? "Sending…" : sendTitle)
                            .font(.headline)
                            .lineLimit(1)
                            .minimumScaleFactor(0.7)
                    }
                } else {
                    HStack(spacing: 6) {
                        Text("Next").font(.headline).lineLimit(1)
                        Image(systemName: "chevron.down").font(.footnote.weight(.bold))
                    }
                }
            }
            // On the floating bar `ButtonRow` gives it what the circles leave of the row, as tall as the row.
            .frame(minWidth: 84, maxWidth: stacked || BottomBar.floats ? .infinity : nil, maxHeight: BottomBar.floats ? .infinity : nil)
        }
        .barPrimaryButton()
        .disabled(reached ? !enabled : (ended || sendingState))
        .accessibilityLabel(label)
        .accessibilityIdentifier(reached ? "send-button" : "next-button")
    }

    /// What "Send" says: an armed instruction is part of what is sent.
    private var sendTitle: LocalizedStringKey {
        switch composer.directive?.kind {
        case .away: "Send and go away"
        case .mute: "Send and stop"
        case nil: "Send"
        }
    }

    /// Beside "Send", while there is more below. On the floating bar a glass circle like the menu's, as tall as the row.
    @ViewBuilder
    private var nextIconButton: some View {
        if BottomBar.floats {
            Button(action: onNext) {
                Image(systemName: "chevron.down").barRowRoundGlyph()
            }
            .barRoundButton()
            .disabled(ended || sendingState)
            .accessibilityLabel("Next question")
            .accessibilityIdentifier("next-button")
        } else {
            Button(action: onNext) {
                Image(systemName: "chevron.down")
                    .font(.body.weight(.semibold))
                    .frame(minWidth: 22)
            }
            .barSecondaryButton()
            .disabled(ended || sendingState)
            .accessibilityLabel("Next question")
            .accessibilityIdentifier("next-button")
        }
    }

    private var sendingState: Bool {
        if case .sending = item.state { return true }
        return false
    }

    private var busy: Bool { composer.isSending || sendingState }

    private var enabled: Bool {
        composer.canSend && !ended && connection == .ready && !sendingState
    }

    private var connectionNotice: RequestConnectionNotice? {
        connection.notice(offlineText: "The Mac is offline. Your draft is kept.")
    }

    /// A line under the count. One that says what "Send" is waiting for also leads to it.
    private var hint: (text: LocalizedStringKey, color: Color, leads: Bool)? {
        if ended { return ("This request has ended.", .secondary, false) }
        if let notice = connectionNotice { return (notice.text, notice.color, false) }
        if sendingState { return ("Waiting for your Mac to confirm…", .secondary, false) }
        if composer.hasReachedEnd, composer.blockedReason == .needsSelection {
            return ("Choose an option for every question.", .secondary, composer.firstWithoutSelection != nil)
        }
        return nil
    }

    /// The hint, as a chip of its own on the floating bar. A hint that leads somewhere is a button, and the
    /// whole chip is what is tapped (its glass is around the button, not in the label: see
    /// `barChipButtonLabel()`).
    @ViewBuilder
    private func hintView(_ hint: (text: LocalizedStringKey, color: Color, leads: Bool), chip: Bool) -> some View {
        let label = HStack(spacing: 3) {
            if !ended, connectionNotice?.progressing == true { ProgressView().controlSize(.mini) }
            Text(hint.text).multilineTextAlignment(.leading)
                .accessibilityIdentifier(!ended && connectionNotice != nil ? "request-connection" : "request-hint")
            if hint.leads { Image(systemName: "chevron.right").font(.caption2.weight(.bold)) }
        }
        .font(.caption)
        .foregroundStyle(hint.color)
        if hint.leads {
            if chip {
                Button(action: onJump) { label.barChipButtonLabel() }
                    .buttonStyle(.plain)
                    .barChipButtonGlass()
                    .accessibilityIdentifier("hint-jump")
            } else {
                Button(action: onJump) { label }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("hint-jump")
            }
        } else if chip {
            label.barStatusChip(compact: true)
        } else {
            label
        }
    }
}

/// The buttons of the floating answer bar: the round ones (the quick-reply menu, "Next") and, last, the main one ("Send").
///
/// All are as tall as the tallest of them, or as the row asks them to be (a count chip that is taller); the round ones are
/// as wide as they are tall, so that they are circles whatever the text size makes the row; the main one has the rest of the
/// width. A stack cannot do that: the width of a circle follows from the height of the row, which a stack learns only after
/// it has divided the width. (Next to "Send" the circles were a different height from it, and "Next" a capsule.)
private struct ButtonRow: Layout {
    static let spacing: CGFloat = 12

    private struct Arrangement {
        var height: CGFloat
        var mainWidth: CGFloat
        var size: CGSize
    }

    private func arrangement(_ proposal: ProposedViewSize, _ subviews: Subviews) -> Arrangement {
        guard let main = subviews.last else { return Arrangement(height: 0, mainWidth: 0, size: .zero) }
        let rounds = subviews.dropLast()
        let mainIdeal = main.sizeThatFits(.unspecified)
        // The round ones are at least the size of a glass circle round its glyph (`BottomBar.rowHeight`).
        var height = max(mainIdeal.height, rounds.map { $0.sizeThatFits(.unspecified).height }.max() ?? 0)
        if let asked = proposal.height, asked.isFinite { height = max(height, asked) }
        let roundsWidth = CGFloat(rounds.count) * (height + Self.spacing)
        let mainWidth: CGFloat
        if let offered = proposal.width, offered.isFinite {
            mainWidth = max(offered - roundsWidth, main.sizeThatFits(.zero).width)
        } else {
            mainWidth = mainIdeal.width
        }
        return Arrangement(height: height, mainWidth: mainWidth, size: CGSize(width: roundsWidth + mainWidth, height: height))
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrangement(proposal, subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard let main = subviews.last else { return }
        let placed = arrangement(ProposedViewSize(width: bounds.width, height: bounds.height), subviews)
        var x = bounds.minX
        for round in subviews.dropLast() {
            round.place(at: CGPoint(x: x, y: bounds.minY), anchor: .topLeading, proposal: ProposedViewSize(width: placed.height, height: placed.height))
            x += placed.height + Self.spacing
        }
        main.place(at: CGPoint(x: x, y: bounds.minY), anchor: .topLeading, proposal: ProposedViewSize(width: placed.mainWidth, height: placed.height))
    }
}

/// The count and the buttons of the floating answer bar: side by side while there is room for both, and the count above the
/// buttons when there is not. The buttons keep their own size either way.
///
/// What this replaces was a plain row in which every block took the height of the tallest. On a narrow screen the count was
/// left a few points of width and wrapped to three or four lines, and the buttons were stretched to that height with it.
///
/// It takes two subviews, in this order: the count chip, which has a short wording of its own for when it is offered little
/// room (so how wide it is depends on the width it is offered), and the row of buttons, which is as wide as it needs to be.
private struct CountAndButtons: Layout {
    /// The least room between the count and the buttons. It is more than the glass container's merge distance (`GlassGroup`
    /// in `SendBar`), so the two shapes never melt into one.
    static let gap: CGFloat = 12
    /// Between the count and the buttons when the count is above them.
    static let stackedSpacing: CGFloat = 10

    /// How the subviews go for a width. Each is placed with the proposal it was measured with (the height aside), so that it
    /// makes the same choices when it is placed as when it was measured: the count's wording depends on the width it is offered.
    private struct Arrangement {
        var sideBySide: Bool
        var countProposal: ProposedViewSize
        var buttonsProposal: ProposedViewSize
        /// The height of the count: where the buttons start when it is above them.
        var countHeight: CGFloat
        var size: CGSize
    }

    private func arrangement(width: CGFloat?, subviews: Subviews) -> Arrangement {
        guard subviews.count == 2 else {
            return Arrangement(sideBySide: true, countProposal: .zero, buttonsProposal: .zero, countHeight: 0, size: .zero)
        }
        let count = subviews[0]
        let buttons = subviews[1]
        // One height for both, as the chip and the capsules of a row have: the taller of the two. (The count is as tall for
        // either wording; it is taller than the buttons while an upload's progress line is in it.)
        let height = max(count.sizeThatFits(.unspecified).height, buttons.sizeThatFits(.unspecified).height)
        // The buttons come first and are never squeezed: the count has what they leave of the width. Their width follows
        // from the height of the row, since the round ones are as wide as they are tall.
        let buttonsSize = buttons.sizeThatFits(ProposedViewSize(width: nil, height: height))
        let room = width.map { max($0 - buttonsSize.width - Self.gap, 0) }
        let countSize = count.sizeThatFits(ProposedViewSize(width: room, height: nil))
        if let width, let room, countSize.width > room + 0.5 {
            // Not even the short wording fits beside the buttons.
            let wide = ProposedViewSize(width: width, height: nil)
            let countAbove = count.sizeThatFits(wide)
            let buttonsBelow = buttons.sizeThatFits(wide)
            return Arrangement(
                sideBySide: false,
                countProposal: wide,
                buttonsProposal: wide,
                countHeight: countAbove.height,
                size: CGSize(width: width, height: countAbove.height + Self.stackedSpacing + buttonsBelow.height)
            )
        }
        return Arrangement(
            sideBySide: true,
            countProposal: ProposedViewSize(width: room, height: height),
            buttonsProposal: ProposedViewSize(width: nil, height: height),
            countHeight: height,
            size: CGSize(width: width ?? countSize.width + Self.gap + buttonsSize.width, height: height)
        )
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrangement(width: proposal.width, subviews: subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard subviews.count == 2 else { return }
        let placed = arrangement(width: bounds.width, subviews: subviews)
        subviews[0].place(at: CGPoint(x: bounds.minX, y: bounds.minY), anchor: .topLeading, proposal: placed.countProposal)
        if placed.sideBySide {
            subviews[1].place(at: CGPoint(x: bounds.maxX, y: bounds.minY), anchor: .topTrailing, proposal: placed.buttonsProposal)
        } else {
            let top = bounds.minY + placed.countHeight + Self.stackedSpacing
            subviews[1].place(at: CGPoint(x: bounds.minX, y: top), anchor: .topLeading, proposal: placed.buttonsProposal)
        }
    }
}
