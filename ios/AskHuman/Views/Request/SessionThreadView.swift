import SwiftUI
import AHCore
import AHMarkdown
import AHProtocol

// The earlier exchanges of a request's session, above the request (docs/specs/ios-session-history.md §4): cards that open
// in place, a row that says why the list ends, and the way back to the request.

/// The thread region of a request page: nothing (not even a gap) until the history was asked for.
///
/// It sits outside everything the page dims or disables once the request has ended: the exchanges of an ended request
/// can still be read.
struct SessionThreadView: View {
    let history: RequestHistory
    private var thread: SessionThread { history.thread }

    var body: some View {
        if history.showsThread {
            let newest = thread.items.last?.id
            VStack(alignment: .leading, spacing: 12) {
                topRow
                ForEach(thread.items) { exchange in
                    SessionExchangeCard(exchange: exchange)
                        .background { if exchange.id == newest { SessionThreadAnchor(name: "thread-newest") } }
                }
                if !thread.items.isEmpty { CurrentRequestDivider() }
            }
            .readableWidth()
            .padding([.horizontal, .top])
            .padding(.bottom, 4)
        }
    }

    @ViewBuilder
    private var topRow: some View {
        if case .failed = thread.phase {
            SessionThreadErrorRow { Task { await history.retry() } }
        } else if thread.reachedStart {
            Text("No earlier records")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity)
                .accessibilityIdentifier("session-thread-start")
        }
    }
}

extension View {
    /// Gives the request room to stay where it is while a thread is above it: see `RequestRoom`.
    func requestRoom(_ history: RequestHistory?) -> some View { modifier(RequestRoom(history: history)) }
}

/// While there is a thread above the request, the request's area is at least as tall as the part of the screen it is seen
/// in.
///
/// A short request has nothing below it to scroll over. Without the room the page could not be scrolled so that the
/// request stays where it was (a pull) or the newest exchange is at the top (the menu): UIKit lets an offset past the end
/// of the content stand, whether it is brought back before the next touch depends on timing, and a page that rests out of
/// its range jumps at the first touch. With it, every place the page is scrolled to is within the range, and a short
/// page looks as it always did: the request at the top, empty below.
///
/// The structure is the same whether or not there is a thread (nothing under it is rebuilt when the thread comes in).
private struct RequestRoom: ViewModifier {
    let history: RequestHistory?

    func body(content: Content) -> some View {
        // Read here, where the view is observed: the frame closure below runs during layout, which observes nothing.
        let room = history?.showsThread ?? false
        ZStack(alignment: .top) {
            Color.clear.containerRelativeFrame(.vertical) { length, _ in room ? length : 0 }
            content
        }
    }
}

/// Where the thread ends and the request itself begins.
private struct CurrentRequestDivider: View {
    var body: some View {
        HStack(spacing: 10) {
            line
            Text("Current request").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            line
        }
        .padding(.top, 4)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
        .accessibilityIdentifier("session-thread-current")
    }

    private var line: some View {
        Capsule().fill(Color.secondary.opacity(0.35)).frame(height: 1).frame(maxWidth: .infinity)
    }
}

/// The Mac could not be reached (offline, or it did not answer): what was loaded stays, and asking again tries once more.
private struct SessionThreadErrorRow: View {
    let retry: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "wifi.exclamationmark").foregroundStyle(.orange)
            Text("Can't reach the Mac").font(.subheadline).foregroundStyle(.secondary)
            Spacer(minLength: 8)
            Button("Try again", action: retry)
                .buttonStyle(.bordered)
                .controlSize(.small)
                .accessibilityIdentifier("session-thread-retry")
        }
        .card(padding: 12)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("session-thread-error")
    }
}

// MARK: a card

/// One exchange: when and where it was answered, what the Agent said, and each question with its answer. A tap opens it in
/// place (the whole message, every question, the counts of what was attached) and closes it again.
private struct SessionExchangeCard: View {
    let exchange: SessionExchange
    @State private var expanded = false
    /// About four lines of the message, whatever the text size.
    @ScaledMetric(relativeTo: .subheadline) private var foldHeight: CGFloat = 84
    /// Questions a folded card names; the rest are counted.
    private static let foldedQuestions = 2

    private var automatic: Bool { SessionChannel.isAutomatic(exchange.channel) }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(verbatim: header).font(.caption).foregroundStyle(.secondary)
            if !exchange.message.isEmpty { message }
            questions
            if expanded { extras }
        }
        .card()
        // An answer the Mac gave by itself (away or mute) is there to explain what the Agent did, not to be weighed.
        .opacity(automatic ? 0.6 : 1)
        .contentShape(Rectangle())
        .onTapGesture { toggle() }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(spoken)
        .accessibilityValue(expanded ? Text("Expanded") : Text("Collapsed"))
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { toggle() }
        .accessibilityIdentifier("session-exchange")
    }

    private func toggle() {
        withAnimation(.easeInOut(duration: 0.2)) { expanded.toggle() }
    }

    // MARK: header

    private var header: String {
        var parts = [SessionTime.label(exchange.at), SessionChannel.name(exchange.channel)]
        if automatic { parts.append(String(localized: "Automatic reply")) }
        return parts.joined(separator: " · ")
    }

    // MARK: message

    @ViewBuilder
    private var message: some View {
        if expanded {
            messageBody(exchange.message)
        } else if exchange.markdown {
            let shown = Self.folded(exchange.message)
            // The whole message when it is short; four lines that fade out when it is not.
            ViewThatFits(in: .vertical) {
                messageBody(shown)
                messageBody(shown)
                    .frame(height: foldHeight, alignment: .top)
                    .clipped()
                    .mask(
                        LinearGradient(
                            stops: [.init(color: .black, location: 0), .init(color: .black, location: 0.65), .init(color: .clear, location: 1)],
                            startPoint: .top, endPoint: .bottom
                        )
                    )
            }
            .frame(maxHeight: foldHeight, alignment: .top)
        } else {
            Text(verbatim: exchange.message).font(.subheadline).lineLimit(4).frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private func messageBody(_ text: String) -> some View {
        if exchange.markdown {
            MarkdownView(text: text, baseFont: .subheadline)
        } else {
            Text(verbatim: text).font(.subheadline).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// A long message is cut before it is drawn: a folded card shows four lines of it, and a report of thousands of
    /// characters would be laid out for nothing.
    private static func folded(_ text: String) -> String {
        guard text.count > 700 else { return text }
        let head = String(text.prefix(600))
        if let cut = head.lastIndex(of: "\n") { return String(head[..<cut]) }
        return head
    }

    // MARK: questions

    @ViewBuilder
    private var questions: some View {
        let all = exchange.qs
        let shown = expanded ? all : Array(all.prefix(Self.foldedQuestions))
        ForEach(Array(shown.enumerated()), id: \.offset) { _, question in
            VStack(alignment: .leading, spacing: 2) {
                questionText(question)
                answer(of: question)
                if expanded { counts(of: question) }
            }
        }
        if !expanded, all.count > Self.foldedQuestions {
            Text("\(all.count - Self.foldedQuestions) more questions").font(.footnote).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private func questionText(_ question: SessionQuestion) -> some View {
        if !expanded {
            Text(verbatim: question.text).font(.subheadline.weight(.medium)).lineLimit(1)
        } else if exchange.markdown {
            MarkdownView(text: question.text, baseFont: .subheadline.weight(.medium))
        } else {
            Text(verbatim: question.text).font(.subheadline.weight(.medium)).textSelection(.enabled)
        }
    }

    @ViewBuilder
    private func answer(of question: SessionQuestion) -> some View {
        let written = question.input.flatMap { $0.isEmpty ? nil : $0 }
        if question.selected.isEmpty, written == nil {
            Text("Not answered").font(.subheadline).foregroundStyle(.tertiary)
        } else if expanded {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Array(question.selected.enumerated()), id: \.offset) { _, option in
                    Text(verbatim: "✓ " + option).font(.subheadline).foregroundStyle(.secondary).textSelection(.enabled)
                }
                if let written {
                    Text(verbatim: written).font(.subheadline).foregroundStyle(.secondary).textSelection(.enabled)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            Text(verbatim: "✓ " + (question.selected + (written.map { [$0] } ?? [])).joined(separator: " · "))
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(2)
        }
    }

    // MARK: what was attached

    @ViewBuilder
    private func counts(of question: SessionQuestion) -> some View {
        let parts = [
            question.images > 0 ? String(localized: "\(Int(question.images)) images") : nil,
            question.files > 0 ? String(localized: "\(Int(question.files)) files") : nil,
        ].compactMap { $0 }
        if !parts.isEmpty {
            Text(verbatim: parts.joined(separator: " · ")).font(.footnote).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var extras: some View {
        let parts = [
            exchange.attachments > 0 ? String(localized: "\(Int(exchange.attachments)) attachments") : nil,
            exchange.notes > 0 ? String(localized: "\(Int(exchange.notes)) annotations") : nil,
        ].compactMap { $0 }
        if !parts.isEmpty {
            Text(verbatim: parts.joined(separator: " · ")).font(.footnote).foregroundStyle(.secondary)
        }
    }

    // MARK: spoken

    /// What VoiceOver reads for the card as a whole: when, where, the message, and each question with its answer.
    private var spoken: String {
        var parts = [header]
        if !exchange.message.isEmpty {
            parts.append(exchange.markdown ? MarkdownParser.plainText(exchange.message) : exchange.message)
        }
        for question in exchange.qs {
            let written = question.input.flatMap { $0.isEmpty ? nil : $0 }
            let given = question.selected + (written.map { [$0] } ?? [])
            parts.append(question.text + ": " + (given.isEmpty ? String(localized: "Not answered") : given.joined(separator: ", ")))
        }
        return parts.joined(separator: ". ")
    }
}

// MARK: names and times

/// Where an exchange was answered (`SessionExchange.channel`): the name the person knows it by. An id the phone does not
/// know is shown as it is.
enum SessionChannel {
    static func name(_ id: String) -> String {
        switch id {
        case "popup": String(localized: "Mac popup")
        case "dingding": String(localized: "DingTalk")
        case "feishu": String(localized: "Feishu")
        case "telegram": "Telegram"
        case "slack": "Slack"
        case "ios": "iPhone"
        case "away": String(localized: "Away mode")
        case "mute": String(localized: "Mute mode")
        default: id
        }
    }

    /// The Mac answered by itself: away mode or mute.
    static func isAutomatic(_ id: String) -> Bool { id == "away" || id == "mute" }
}

private enum SessionTime {
    private static let formatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        // "Yesterday, 9:40 PM": the day names are the ones of the language.
        formatter.doesRelativeDateFormatting = true
        return formatter
    }()

    static func label(_ ms: Int64) -> String {
        formatter.string(from: Date(timeIntervalSince1970: Double(ms) / 1000))
    }
}

// MARK: floating

/// "Back to the current request", while the page is more than a screen above it.
struct SessionReturnCapsule: View {
    let history: RequestHistory

    var body: some View {
        ZStack {
            if history.farAbove {
                Button { history.returnToCurrent() } label: {
                    Label("Back to current request", systemImage: "arrow.down")
                        .font(.subheadline.weight(.semibold))
                        .padding(.horizontal, 14)
                        .padding(.vertical, 9)
                }
                .buttonStyle(.plain)
                .background(.regularMaterial, in: Capsule())
                .overlay(Capsule().strokeBorder(Color.secondary.opacity(0.25)))
                .shadow(color: .black.opacity(0.12), radius: 5, y: 1)
                .padding(.bottom, 12)
                .transition(.move(edge: .bottom).combined(with: .opacity))
                .accessibilityIdentifier("session-return")
            }
        }
        .animation(.easeOut(duration: 0.2), value: history.farAbove)
    }
}

/// The history is being loaded from the menu: the page does not move while it comes in, so this floats over it.
struct SessionLoadingPill: View {
    let history: RequestHistory

    var body: some View {
        ZStack {
            if history.menuLoading {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Loading history…").font(.subheadline)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(.regularMaterial, in: Capsule())
                .shadow(color: .black.opacity(0.12), radius: 5, y: 1)
                .padding(.top, 8)
                .transition(.move(edge: .top).combined(with: .opacity))
                .allowsHitTesting(false)
                .accessibilityIdentifier("session-loading")
            }
        }
        .animation(.easeOut(duration: 0.2), value: history.menuLoading)
    }
}
