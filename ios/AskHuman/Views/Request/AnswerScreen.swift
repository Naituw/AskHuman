import SwiftUI
import AHCore
import AHProtocol
import AHStore

/// The long form for a question request: message, one card per question, a fixed bottom bar.
///
/// With several questions the bar offers "Next" until the last question has been on screen, and
/// only then "Send", as the Mac popup does: a stray tap cannot send answers that were never read.
struct AnswerScreen: View {
    let item: InboxItem
    @Environment(AppStore.self) private var store
    @State private var confirmCancel = false
    @State private var problem: String?
    @State private var tracker = ReadingTracker()

    fileprivate static let space = "answer-scroll"

    var body: some View {
        if let ask = item.view?.ask, let composer = store.composer(for: item) {
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        if let finished = store.finished[item.id], store.receipt?.from != item.id { FinishedNotice(finished: finished) }
                        if case .failed = item.state { FailedSubmissionNotice(item: item) }
                        RequestHeader(item: item)
                        MessageCard(ask: ask, macId: item.macId)
                        VStack(spacing: 0) {
                            ForEach(Array(ask.questions.enumerated()), id: \.offset) { index, question in
                                QuestionCard(index: index, count: ask.questions.count, ask: ask, question: question, composer: composer)
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
                        item: item, composer: composer, ended: store.finished[item.id] != nil,
                        online: store.isOnline(macId: item.macId), onSend: { send(composer) }, onNext: { next(composer, proxy: proxy) }
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

    /// Scrolls the first question that was not read yet into view: all of it when it fits on the
    /// screen (as the popup's "next" does), else its top.
    private func next(_ composer: AnswerComposer, proxy: ScrollViewProxy) {
        guard let target = composer.nextUnread else { return }
        let fits = tracker.fits(target)
        withAnimation(.easeInOut(duration: 0.4)) {
            proxy.scrollTo(Self.cardID(target), anchor: fits ? .bottom : .top)
        }
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

/// Marks the questions that were on screen. A plain object on purpose: its numbers change on every
/// frame of a scroll and must not make the screen render again; only a newly seen question does.
@MainActor
private final class ReadingTracker {
    private var latest = Reading()
    private var live = false

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
        if live { markSeen(composer) }
    }

    /// Whether a whole question fits between the bars.
    func fits(_ question: Int) -> Bool {
        guard let barTop = latest.barTop, let frame = latest.cards[question] else { return true }
        return frame.height <= barTop - (latest.safeTop ?? 0) - 16
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
                .padding(10)
                .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityIdentifier("answer-text-\(index)")
                ReplyAttachmentBar(composer: composer, question: index)
            }
        }
        .card()
    }
}

private struct OptionRow: View {
    let option: OptionView
    let single: Bool
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: symbol)
                    .font(.title3)
                    .foregroundStyle(selected ? Color.accentColor : Color(.tertiaryLabel))
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
            .background(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(selected ? Color.accentColor.opacity(0.12) : Color(.tertiarySystemFill))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(selected ? Color.accentColor : .clear, lineWidth: 1.5)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var symbol: String {
        if single { return selected ? "largecircle.fill.circle" : "circle" }
        return selected ? "checkmark.square.fill" : "square"
    }
}

// MARK: send bar

private struct SendBar: View {
    let item: InboxItem
    let composer: AnswerComposer
    let ended: Bool
    let online: Bool
    let onSend: () -> Void
    let onNext: () -> Void

    var body: some View {
        VStack(spacing: 8) {
            if let progress = composer.uploadProgress {
                ProgressView(value: progress).progressViewStyle(.linear)
            }
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Answered \(composer.answeredCount)/\(composer.questionCount)")
                        .font(.subheadline.weight(.medium))
                        .accessibilityIdentifier("answered-count")
                    if let hint {
                        Text(hint.text).font(.caption).foregroundStyle(hint.color)
                    }
                }
                Spacer(minLength: 8)
                // Sending waits until the last question was on screen; until then the button leads there.
                if composer.hasReachedEnd {
                    sendButton.transition(.opacity.combined(with: .scale(scale: 0.92)))
                } else {
                    nextButton.transition(.opacity.combined(with: .scale(scale: 0.92)))
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
        .animation(.snappy(duration: 0.25), value: composer.hasReachedEnd)
    }

    private var sendButton: some View {
        Button(action: onSend) {
            HStack(spacing: 6) {
                if busy { ProgressView().controlSize(.small).tint(.white) }
                Text(busy ? "Sending…" : "Send")
                    .font(.headline)
            }
            .frame(minWidth: 84)
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.large)
        .disabled(!enabled)
        .accessibilityIdentifier("send-button")
    }

    private var nextButton: some View {
        Button(action: onNext) {
            HStack(spacing: 6) {
                Text("Next").font(.headline)
                Image(systemName: "chevron.down").font(.footnote.weight(.bold))
            }
            .frame(minWidth: 84)
        }
        .buttonStyle(.borderedProminent)
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

    private var hint: (text: LocalizedStringKey, color: Color)? {
        if ended { return ("This request has ended.", .secondary) }
        if !online { return ("The Mac is offline. Your draft is kept.", .orange) }
        if sendingState { return ("Waiting for your Mac to confirm…", .secondary) }
        if composer.hasReachedEnd, composer.blockedReason == .needsSelection { return ("Choose an option for every question.", .secondary) }
        return nil
    }
}
