import SwiftUI
import AHCore
import AHProtocol
import AHStore

/// The long form for a question request: message, one card per question, a fixed bottom bar.
///
/// With several questions the bar offers "Next" until the last question has been on screen, and
/// only then "Send", as the Mac popup does: a stray tap cannot send answers that were never read.
/// "Next" itself follows where the screen is (`QuestionNavigator`), never what was looked at before,
/// so that a question that was seen but not answered is not jumped over.
struct AnswerScreen: View {
    let item: InboxItem
    @Environment(AppStore.self) private var store
    @State private var confirmCancel = false
    @State private var problem: String?
    @State private var tracker = ReadingTracker()
    /// The question "Next" last led to, which flashes once it has arrived.
    @State private var landing = Landing()
    /// The question whose text field has the keyboard.
    @FocusState private var focused: Int?

    fileprivate static let space = "answer-scroll"

    var body: some View {
        if let ask = item.view?.ask, let composer = store.composer(for: item) {
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        if let finished = store.endedNotice(for: item.id) { FinishedNotice(finished: finished) }
                        if case .failed = item.state { FailedSubmissionNotice(item: item) }
                        RequestHeader(item: item)
                        MessageCard(ask: ask, macId: item.macId)
                        VStack(spacing: 0) {
                            ForEach(Array(ask.questions.enumerated()), id: \.offset) { index, question in
                                QuestionCard(
                                    index: index, count: ask.questions.count, ask: ask, question: question, composer: composer,
                                    landing: landing, focus: $focused
                                )
                                    .background(FrameReader(.card(index)))
                                    // The gap belongs to what "Next" scrolls to, so that a card does not end flush with the bar.
                                    .padding(.bottom, 16)
                                    .id(Self.cardID(index))
                            }
                        }
                    }
                    .padding([.horizontal, .top])
                    .padding(.bottom, 8)
                }
                .background(Color(.systemGroupedBackground))
                .background(FrameReader(.safeTop))
                .scrollDismissesKeyboard(.interactively)
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    SendBar(
                        item: item, composer: composer, moreBelow: tracker.moreBelow, ended: store.endedNotice(for: item.id) != nil,
                        online: store.isOnline(macId: item.macId),
                        onSend: { send(composer) },
                        onNext: { next(composer, proxy: proxy) },
                        onJump: { jumpToMissing(composer, proxy: proxy) }
                    )
                    .background(FrameReader(.barTop))
                }
                .coordinateSpace(name: Self.space)
                .onPreferenceChange(ReadingKey.self) { tracker.update($0, composer: composer) }
                .task(id: item.id) { await tracker.begin(composer) }
            }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Button("Cancel request", systemImage: "xmark.circle", role: .destructive) { confirmCancel = true }
                            .disabled(store.finished[item.id] != nil || !store.isOnline(macId: item.macId))
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                    .accessibilityIdentifier("request-menu")
                }
            }
            .confirmationDialog("Cancel this request?", isPresented: $confirmCancel, titleVisibility: .visible) {
                Button("Cancel request", role: .destructive) {
                    Task {
                        do { try await store.cancel(item) } catch { problem = error.userMessage }
                    }
                }
                Button("Keep it", role: .cancel) {}
            } message: {
                Text("The Agent is told that you cancelled.")
            }
            .alert("Could not send", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(problem ?? "")
            }
        }
    }

    fileprivate static func cardID(_ index: Int) -> String { "question-\(index)" }

    private func send(_ composer: AnswerComposer) {
        Task {
            do {
                try await store.send(composer, item: item)
            } catch {
                problem = error.userMessage
            }
        }
    }

    /// Leads to the first question whose bottom is not on screen yet: all of it when it fits (as the
    /// popup's "next" does), else its top, and on the next press its bottom. Nothing that was seen
    /// or answered before matters, only where the screen is now.
    private func next(_ composer: AnswerComposer, proxy: ScrollViewProxy) {
        Task {
            await putTheKeyboardAway()
            guard let viewport = tracker.viewport(),
                  let target = QuestionNavigator.next(in: viewport, count: composer.questionCount) else { return }
            go(to: target, proxy: proxy)
        }
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
        withAnimation(.easeInOut(duration: 0.4)) {
            proxy.scrollTo(Self.cardID(target.question), anchor: target.anchor == .bottom ? .bottom : .top)
        }
        landing.flash(target.question)
    }

    /// The keyboard covers the question that is being led to, and its coming down changes the room
    /// there is: let it go first, and the scroll is worked out for the screen that is left.
    private func putTheKeyboardAway() async {
        guard focused != nil else { return }
        focused = nil
        try? await Task.sleep(for: .milliseconds(320))
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
    /// Where the bottom bar starts: what is above it is on screen.
    var barTop: CGFloat?
    /// Where the navigation bar ends.
    var safeTop: CGFloat?
}

private struct ReadingKey: PreferenceKey {
    static var defaultValue = Reading()

    static func reduce(value: inout Reading, nextValue: () -> Reading) {
        let next = nextValue()
        value.cards.merge(next.cards) { $1 }
        if let barTop = next.barTop { value.barTop = barTop }
        if let safeTop = next.safeTop { value.safeTop = safeTop }
    }
}

private struct FrameReader: View {
    enum Part {
        case card(Int)
        case barTop
        case safeTop
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
        case .barTop:
            return Reading(barTop: proxy.frame(in: .named(AnswerScreen.space)).minY)
        case .safeTop:
            return Reading(safeTop: proxy.safeAreaInsets.top)
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
    @ObservationIgnored private var questionCount = 0

    /// The first layout passes are not trusted (the bars may not have their final size yet): the
    /// questions on screen are counted a moment after the form appeared.
    func begin(_ composer: AnswerComposer) async {
        live = false
        try? await Task.sleep(for: .milliseconds(150))
        guard !Task.isCancelled else { return }
        live = true
        markSeen(composer)
    }

    func update(_ reading: Reading, composer: AnswerComposer) {
        latest = reading
        questionCount = composer.questionCount
        if live { markSeen(composer) }
        let more = viewport().flatMap { QuestionNavigator.next(in: $0, count: questionCount) } != nil
        if more != moreBelow { moreBelow = more }
    }

    /// What the screen shows of the questions right now; `nil` before anything was measured.
    func viewport() -> QuestionNavigator.Viewport? {
        guard let barTop = latest.barTop else { return nil }
        return QuestionNavigator.Viewport(cards: latest.cards, top: latest.safeTop ?? 0, bottom: barTop)
    }

    /// A question counts as seen once its bottom is above the bottom bar, whether it is still on
    /// screen or already scrolled past (spec `multi-question-vertical.md` V1).
    private func markSeen(_ composer: AnswerComposer) {
        guard let barTop = latest.barTop else { return }
        for (index, frame) in latest.cards where frame.maxY <= barTop + 1 {
            composer.markSeen(index)
        }
    }
}

// MARK: message

private struct MessageCard: View {
    let ask: AskView
    let macId: String

    var body: some View {
        let text = ask.message.text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !text.isEmpty || !ask.message.files.isEmpty {
            VStack(alignment: .leading, spacing: 12) {
                if !text.isEmpty {
                    if ask.isMarkdown {
                        MarkdownView(text: text)
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
    var focus: FocusState<Int?>.Binding

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
                VStack(spacing: 8) {
                    ForEach(Array(question.options.enumerated()), id: \.offset) { optionIndex, option in
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
            if !composer.selectOnly {
                TextField(
                    question.options.isEmpty ? "Your answer" : "Add a note (optional)",
                    text: Binding(get: { composer.answers[index].text }, set: { composer.setText($0, question: index) }),
                    axis: .vertical
                )
                .lineLimit(1...10)
                .focused(focus, equals: index)
                .padding(10)
                .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityIdentifier("answer-text-\(index)")
                ReplyAttachmentBar(composer: composer, question: index)
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
private struct OptionRow: View {
    let option: OptionView
    let single: Bool
    let selected: Bool
    let action: () -> Void

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
                Text(verbatim: option.text)
                    .font(.body)
                    .foregroundStyle(.primary)
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if option.recommended {
                    Chip(text: "Recommended", systemImage: "star.fill", tint: .orange)
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
    let online: Bool
    let onSend: () -> Void
    let onNext: () -> Void
    let onJump: () -> Void

    @Environment(\.dynamicTypeSize) private var typeSize

    /// At the accessibility text sizes the buttons go under the text: side by side they would leave
    /// the words a few letters of width.
    private var stacked: Bool { typeSize.isAccessibilitySize }

    var body: some View {
        VStack(spacing: 8) {
            if let progress = composer.uploadProgress {
                ProgressView(value: progress).progressViewStyle(.linear)
            }
            let layout = stacked ? AnyLayout(VStackLayout(alignment: .leading, spacing: 10)) : AnyLayout(HStackLayout(spacing: 12))
            layout {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Answered \(composer.answeredCount)/\(composer.questionCount)")
                        .font(.subheadline.weight(.medium))
                        .accessibilityIdentifier("answered-count")
                    if let hint {
                        hintView(hint)
                    }
                }
                if !stacked { Spacer(minLength: 8) }
                HStack(spacing: 12) {
                    // Sending waits until the last question was on screen; until then the button leads
                    // there. After that "Send" takes the main place, and "Next" stays beside it while there
                    // are questions below the screen, as the popup's "Next" does until it reaches the last.
                    if composer.hasReachedEnd {
                        if moreBelow {
                            nextIconButton.transition(.opacity.combined(with: .scale(scale: 0.92)))
                        }
                        sendButton.transition(.opacity.combined(with: .scale(scale: 0.92)))
                    } else {
                        nextButton.transition(.opacity.combined(with: .scale(scale: 0.92)))
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
        .animation(.snappy(duration: 0.25), value: composer.hasReachedEnd)
        .animation(.snappy(duration: 0.25), value: moreBelow)
    }

    private var sendButton: some View {
        Button(action: onSend) {
            HStack(spacing: 6) {
                if busy { ProgressView().controlSize(.small).tint(.white) }
                Text(busy ? "Sending…" : "Send")
                    .font(.headline)
                    .lineLimit(1)
            }
            .frame(minWidth: 84, maxWidth: stacked ? .infinity : nil)
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.large)
        .disabled(!enabled)
        .accessibilityIdentifier("send-button")
    }

    /// The main button until the last question was reached.
    private var nextButton: some View {
        Button(action: onNext) {
            HStack(spacing: 6) {
                Text("Next").font(.headline).lineLimit(1)
                Image(systemName: "chevron.down").font(.footnote.weight(.bold))
            }
            .frame(minWidth: 84, maxWidth: stacked ? .infinity : nil)
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.large)
        .disabled(ended || sendingState)
        .accessibilityLabel("Next question")
        .accessibilityIdentifier("next-button")
    }

    /// Beside "Send", while there is more below.
    private var nextIconButton: some View {
        Button(action: onNext) {
            Image(systemName: "chevron.down")
                .font(.body.weight(.semibold))
                .frame(minWidth: 22)
        }
        .buttonStyle(.bordered)
        .controlSize(.large)
        .disabled(ended || sendingState)
        .accessibilityLabel("Next question")
        .accessibilityIdentifier("next-button")
    }

    private var sendingState: Bool {
        if case .sending = item.state { return true }
        return false
    }

    private var busy: Bool { composer.isSending || sendingState }

    private var enabled: Bool {
        composer.canSend && !ended && online && !sendingState
    }

    /// A line under the count. One that says what "Send" is waiting for also leads to it.
    private var hint: (text: LocalizedStringKey, color: Color, leads: Bool)? {
        if ended { return ("This request has ended.", .secondary, false) }
        if !online { return ("The Mac is offline. Your draft is kept.", .orange, false) }
        if sendingState { return ("Waiting for your Mac to confirm…", .secondary, false) }
        if composer.hasReachedEnd, composer.blockedReason == .needsSelection {
            return ("Choose an option for every question.", .secondary, composer.firstWithoutSelection != nil)
        }
        return nil
    }

    @ViewBuilder
    private func hintView(_ hint: (text: LocalizedStringKey, color: Color, leads: Bool)) -> some View {
        if hint.leads {
            Button(action: onJump) {
                HStack(spacing: 3) {
                    Text(hint.text).multilineTextAlignment(.leading)
                    Image(systemName: "chevron.right").font(.caption2.weight(.bold))
                }
                .font(.caption)
                .foregroundStyle(hint.color)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("hint-jump")
        } else {
            Text(hint.text).font(.caption).foregroundStyle(hint.color)
        }
    }
}
