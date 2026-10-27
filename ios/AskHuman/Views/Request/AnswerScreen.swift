import SwiftUI
import AHCore
import AHProtocol
import AHStore

/// The long form for a question request: message, one card per question, a fixed send bar.
struct AnswerScreen: View {
    let item: InboxItem
    @Environment(AppStore.self) private var store
    @State private var confirmCancel = false
    @State private var problem: String?
    @State private var sentTick = 0

    var body: some View {
        if let ask = item.view?.ask, let composer = store.composer(for: item) {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let finished = store.finished[item.id] { FinishedNotice(finished: finished) }
                    if case .failed = item.state { FailedSubmissionNotice(item: item) }
                    RequestHeader(item: item)
                    MessageCard(ask: ask, macId: item.macId)
                    ForEach(Array(ask.questions.enumerated()), id: \.offset) { index, question in
                        QuestionCard(index: index, count: ask.questions.count, ask: ask, question: question, composer: composer)
                    }
                }
                .padding()
                .padding(.bottom, 8)
            }
            .background(Color(.systemGroupedBackground))
            .scrollDismissesKeyboard(.interactively)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                SendBar(
                    item: item, composer: composer, ended: store.finished[item.id] != nil, online: store.isOnline(macId: item.macId)
                ) {
                    send(composer)
                }
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
            .sensoryFeedback(.success, trigger: sentTick)
        }
    }

    private func send(_ composer: AnswerComposer) {
        Task {
            do {
                try await store.send(composer, item: item)
                sentTick += 1
            } catch {
                problem = error.userMessage
            }
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
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
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
        if composer.blockedReason == .needsSelection { return ("Choose an option for every question.", .secondary) }
        return nil
    }
}
