import SwiftUI
import UIKit
import AHCore
import AHMarkdown
import AHProtocol

/// One request page of the inbox stack.
///
/// After a send the next request takes this page's place (`AppStore.advance`): the stack keeps its one
/// page, what it shows changes with `id`, and the new request slides in over the receipt of the old one.
/// The system's own push is not used for that. It needs the sent page kept underneath until the push is
/// over and dropped then, and SwiftUI answers the drop by showing that page's last frame again (the
/// receipt flashes by) and by giving the next request the scroll position of the one before it. Here
/// every request gets a page of its own.
struct RequestScreen: View {
    let id: String
    @Environment(AppStore.self) private var store
    @State private var switching = false
    @State private var confirmCancel = false
    @State private var confirmDismiss = false

    private var hasNeighbors: Bool {
        store.adjacentRequest(to: id, direction: -1) != nil || store.adjacentRequest(to: id, direction: 1) != nil
    }

    private var hasMore: Bool {
        guard let item = store.displayItem(id: id) else { return false }
        return item.view?.ask != nil || item.view?.confirm?.dismissIndex != nil
    }

    var body: some View {
        // The receipt of the request this page takes over from, if it does: the store says so when it
        // moves on. (A page opened any other way has no business with a receipt.)
        let handedOver = store.receipt.flatMap { $0.next == id ? $0 : nil }
        Arrival(from: handedOver) {
            RequestPage(id: id)
        }
        // A page of its own for every request: nothing of the last one (its scroll position, what was
        // read of it) is left on this one.
        .id(id)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .toolbar {
            if hasNeighbors { navigationButtons }
            if hasMore {
                if #available(iOS 26, *), hasNeighbors {
                    ToolbarSpacer(.fixed, placement: .topBarTrailing)
                }
                moreMenu
            }
        }
        .confirmationDialog("Cancel this request?", isPresented: $confirmCancel, titleVisibility: .visible) {
            Button("Cancel request", role: .destructive) {
                guard let item = store.displayItem(id: id) else { return }
                Task { do { try await store.cancel(item) } catch { store.reportRequestError(error.userMessage, for: item.id) } }
            }
            Button("Keep it", role: .cancel) {}
        } message: {
            Text("The Agent is told that you cancelled.")
        }
        .confirmationDialog("Dismiss this confirmation?", isPresented: $confirmDismiss, titleVisibility: .visible) {
            Button("Dismiss", role: .destructive) {
                guard let item = store.displayItem(id: id), let composer = store.confirmComposer(for: item) else { return }
                Task { do { try await store.dismiss(composer, item: item) } catch { store.reportRequestError(error.userMessage, for: item.id) } }
            }
            Button("Keep it", role: .cancel) {}
        } message: {
            Text("The Agent receives the default response, usually a refusal.")
        }
        .alert("Could not send", isPresented: Binding(get: { store.requestErrors[id] != nil }, set: { if !$0 { store.clearRequestError(for: id) } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(store.requestErrors[id] ?? "")
        }
        .onChange(of: id) { _, _ in
            switching = false
            confirmCancel = false
            confirmDismiss = false
        }
        .task(id: id) {
            if let item = store.item(id: id) { await store.opened(item) }
        }
    }

    @ToolbarContentBuilder
    private var navigationButtons: some ToolbarContent {
        ToolbarItemGroup(placement: .topBarTrailing) {
            Button { switchRequest(-1) } label: { Label("Previous request", systemImage: "chevron.backward") }
                .disabled(switching || store.adjacentRequest(to: id, direction: -1) == nil)
                .accessibilityIdentifier("previous-request")
            Button { switchRequest(1) } label: { Label("Next request", systemImage: "chevron.forward") }
                .disabled(switching || store.adjacentRequest(to: id, direction: 1) == nil)
                .accessibilityIdentifier("next-request")
        }
    }

    @ToolbarContentBuilder
    private var moreMenu: some ToolbarContent {
        ToolbarItem(placement: .topBarTrailing) {
            Menu {
                if let item = store.displayItem(id: id) {
                    if item.view?.ask != nil {
                        Button("Cancel request", systemImage: "xmark.circle", role: .destructive) { confirmCancel = true }
                            .disabled(store.finished[id] != nil || !store.isOnline(macId: item.macId))
                    } else {
                        Button("Dismiss", systemImage: "xmark.circle", role: .destructive) { confirmDismiss = true }
                            .disabled(store.finished[id] != nil || !store.isOnline(macId: item.macId))
                    }
                }
            } label: { Image(systemName: "ellipsis") }
            .accessibilityLabel("More")
            .accessibilityIdentifier("request-menu")
        }
    }

    private func switchRequest(_ direction: Int) {
        guard !switching else { return }
        switching = true
        let current = id
        let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows)
        if let window = windows.first(where: \.isKeyWindow) {
            func editingView(_ view: UIView) -> UIView? {
                if view.isFirstResponder { return view }
                for child in view.subviews { if let found = editingView(child) { return found } }
                return nil
            }
            (editingView(window) as? UITextInput)?.unmarkText()
            window.endEditing(true)
        }
        Task { @MainActor in
            // Let native editors publish their final composition before the old page is saved.
            await Task.yield()
            withAnimation(.easeOut(duration: 0.18)) { store.switchRequest(from: current, direction: direction) }
            switching = false
        }
    }
}

/// A request page that takes over from the page that was just sent: the receipt of that page stays
/// where it is, and the new page slides in over it while the receipt gives way a little to the left,
/// as in the system's push.
///
/// This is not a view transition: the old page (it was replaced, and its request is closed) is not
/// kept around to slide away. A transition would keep the old page's whole view tree alive and keep it
/// up to date, and it ran the receipt's own transitions along: the receipt faded out (or was gone at
/// once) and the closed form behind it showed through. The receipt is shown again here, standing by
/// itself, and moved with the page.
private struct Arrival<Content: View>: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// The receipt still on screen, until the new page has come to rest.
    @State private var leaving: SentReceipt?
    @State private var arrived: Bool
    private let content: Content

    /// The page keeps the receipt it started with: it is a page of its own for every request (`.id`).
    init(from: SentReceipt?, @ViewBuilder content: () -> Content) {
        _leaving = State(initialValue: from)
        _arrived = State(initialValue: from == nil)
        self.content = content()
    }

    var body: some View {
        ZStack {
            if let leaving {
                SentReceiptPage(receipt: leaving, settled: true)
                    .visualEffect { view, proxy in view.offset(x: !reduceMotion && arrived ? -proxy.size.width * 0.3 : 0) }
                    .opacity(reduceMotion && arrived ? 0 : 1)
            }
            content
                .visualEffect { view, proxy in view.offset(x: !reduceMotion && !arrived ? proxy.size.width : 0) }
                .opacity(reduceMotion && !arrived ? 0 : 1)
        }
        .task {
            guard leaving != nil, !arrived else { return }
            // The page lays itself out for the first time right after it was added, and for a long form
            // that takes a good part of the slide: an animation that started before it would be seen
            // from the middle, or not at all. Whatever else is waiting for the main thread is done by
            // the time this resumes.
            do { try await Task.sleep(for: .milliseconds(60)) } catch { return }
            // A curve with a duration of its own, not a spring, so that the end is known. It starts
            // slowly and comes to rest softly, much like the spring the system's push is made of.
            let curve: Animation = reduceMotion ? .easeOut(duration: 0.2) : .timingCurve(0.3, 0, 0.2, 1, duration: 0.45)
            withAnimation(curve) {
                arrived = true
            } completion: {
                leaving = nil
            }
            // The receipt must not outlive the slide if the completion does not come.
            do { try await Task.sleep(for: .seconds(2)) } catch { return }
            leaving = nil
        }
    }
}

/// What one request page shows: the form, or what is known of the request while the rest loads, and
/// over it the receipt of the request that was just sent.
private struct RequestPage: View {
    let id: String
    @Environment(AppStore.self) private var store

    /// The receipt of this page, while it shows one.
    private var receipt: SentReceipt? {
        store.receipt?.from == id ? store.receipt : nil
    }

    var body: some View {
        Group {
            if let item = store.displayItem(id: id) {
                if item.view?.ask != nil {
                    AnswerScreen(item: item)
                } else if item.view?.confirm != nil {
                    ConfirmScreen(item: item)
                } else {
                    LoadingRequest(item: item)
                }
            } else {
                MissingRequestPage(id: id)
            }
        }
        // The form takes the placeholder's place when the request arrives.
        .animation(.easeOut(duration: 0.2), value: store.displayItem(id: id) != nil)
        // The receipt takes the form's place for the moment between "the Mac took it" and the next page.
        .accessibilityHidden(receipt != nil)
        .overlay {
            if let receipt { SentReceiptPage(receipt: receipt) }
        }
    }
}

/// What a page shows once the Mac took the reply: a plain page of its own, so that what follows
/// (the next request sliding in) reads as a new page and not as the same form changing.
private struct SentReceiptPage: View {
    let receipt: SentReceipt
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// The page itself fades in over the form it replaces, and the symbol and the text come after it.
    @State private var visible: Bool
    @State private var shown: Bool

    /// `settled` shows the page as it is when it has been up for a while (the copy that slides away).
    init(receipt: SentReceipt, settled: Bool = false) {
        self.receipt = receipt
        _visible = State(initialValue: settled)
        _shown = State(initialValue: settled)
    }

    var body: some View {
        ZStack {
            Color(.systemGroupedBackground).ignoresSafeArea()
            VStack(spacing: 16) {
                Image(systemName: receipt.outcome.symbol)
                    .font(.system(size: 72))
                    .foregroundStyle(receipt.outcome.color)
                    .symbolEffect(.bounce, options: .nonRepeating, value: shown)
                    .scaleEffect(shown || reduceMotion ? 1 : 0.5)
                    .opacity(shown || reduceMotion ? 1 : 0)
                VStack(spacing: 6) {
                    title.font(.title2.weight(.semibold))
                    if let summary = receipt.summary {
                        HStack(spacing: 8) {
                            AgentBadge(agent: summary.agent, size: 22)
                            Text(verbatim: summary.agent.label + (summary.project.name.isEmpty ? "" : " · " + summary.project.name))
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                    }
                }
                .opacity(shown || reduceMotion ? 1 : 0)
                .offset(y: shown || reduceMotion ? 0 : 8)
            }
            .padding(.horizontal, 32)
        }
        .opacity(visible ? 1 : 0)
        .onAppear {
            guard !visible else { return }
            withAnimation(.easeOut(duration: 0.2)) { visible = true }
            withAnimation(reduceMotion ? nil : .spring(duration: 0.5, bounce: 0.35)) { shown = true }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("sent-receipt")
    }

    private var title: Text {
        switch receipt.outcome {
        case .answered: Text("Sent")
        case .cancelled: Text("Cancelled")
        case .dismissed: Text("Dismissed")
        default: Text(receipt.outcome.title(by: nil))
        }
    }
}

/// The page of a request the phone does not have: on its way (a tapped notification is ahead of the
/// request itself), out of reach, or gone. Only the last says "unavailable".
private struct MissingRequestPage: View {
    let id: String
    @Environment(AppStore.self) private var store

    var body: some View {
        let missing = store.missingRequest(id: id)
        if missing == .gone {
            ContentUnavailableView("Request unavailable", systemImage: "tray", description: Text("It was answered or cancelled."))
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let hint = store.hint(for: id) { HintCard(hint: hint) }
                    status(missing)
                }
                .padding()
            }
            .background(Color(.systemGroupedBackground))
            .accessibilityIdentifier("request-placeholder")
        }
    }

    @ViewBuilder
    private func status(_ missing: MissingRequest) -> some View {
        if missing == .unreachable {
            Label {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Your Mac is not reachable").font(.subheadline.weight(.semibold))
                    Text("The request shows up here when it is back, if it is still waiting.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
            } icon: {
                Image(systemName: "wifi.exclamationmark").foregroundStyle(.orange)
            }
            .card()
        } else {
            HStack(spacing: 8) {
                ProgressView()
                Text("Loading the full request…").foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// What the notification said: who asks, from which Mac, and the start of the question.
private struct HintCard: View {
    let hint: NotificationHint

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(verbatim: hint.title).font(.headline)
            if !hint.subtitle.isEmpty {
                Text(verbatim: hint.subtitle).font(.footnote).foregroundStyle(.secondary)
            }
            if !hint.body.isEmpty {
                Text(verbatim: hint.body).font(.body).padding(.top, 6)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .card()
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("request-placeholder-hint")
    }
}

private struct LoadingRequest: View {
    let item: InboxItem

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                RequestHeader(item: item)
                Text(MarkdownParser.plainText(item.summary.preview)).font(.body)
                HStack(spacing: 8) {
                    ProgressView()
                    Text("Loading the full request…").foregroundStyle(.secondary)
                }
            }
            .padding()
        }
        .background(Color(.systemGroupedBackground))
    }
}

/// Who asks, about which project, on which Mac, and when.
struct RequestHeader: View {
    let item: InboxItem

    var body: some View {
        HStack(spacing: 12) {
            AgentBadge(agent: item.summary.agent, size: 44)
            VStack(alignment: .leading, spacing: 2) {
                if let title = item.sessionTitle {
                    // The session's title leads; who asks and where from moves into a small line beneath it.
                    Text(verbatim: title)
                        .font(.headline)
                        .lineLimit(2)
                        .accessibilityIdentifier("request-title")
                    Text(verbatim: [item.summary.agent.label, item.summary.project.name, item.macName].filter { !$0.isEmpty }.joined(separator: " · "))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .accessibilityIdentifier("request-origin")
                } else {
                    Text(item.summary.agent.label).font(.headline)
                    HStack(spacing: 5) {
                        if !item.summary.project.name.isEmpty {
                            Image(systemName: "folder.fill").imageScale(.small)
                            Text(item.summary.project.name)
                            Text("·")
                        }
                        Image(systemName: "desktopcomputer").imageScale(.small)
                        Text(item.macName)
                    }
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                }
                Text(Date(timeIntervalSince1970: TimeInterval(item.summary.createdAt) / 1000), format: .dateTime.month().day().hour().minute())
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            Spacer(minLength: 0)
        }
    }
}

/// The notice on a page whose request ended while it was open.
struct FinishedNotice: View {
    let finished: FinishedRequest

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text("This request has ended").font(.subheadline.weight(.semibold))
                Text(detail).font(.footnote).foregroundStyle(.secondary)
            }
        } icon: {
            Image(systemName: finished.outcome.symbol).foregroundStyle(finished.outcome.color)
        }
        .card()
        .accessibilityIdentifier("finished-notice")
    }

    private var detail: String {
        switch finished.outcome {
        case .answeredElsewhere: finished.by.map { String(localized: "It was answered on \($0).") } ?? String(localized: "It was answered elsewhere.")
        case .cancelled: String(localized: "The Agent cancelled it.")
        case .expired: String(localized: "It timed out.")
        case .answered: String(localized: "It was answered.")
        case .dismissed: String(localized: "It was dismissed.")
        case .ended: String(localized: "It finished while you were away.")
        }
    }
}

/// Shown when the last submission did not get through.
struct FailedSubmissionNotice: View {
    let item: InboxItem
    @Environment(AppStore.self) private var store

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Your reply was not delivered", systemImage: "exclamationmark.triangle.fill")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.red)
            Text("The Mac did not confirm it. You can send it again or discard it and edit your answer.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            HStack {
                Button("Send again") {
                    Task {
                        do { try await store.retry(item) } catch { store.reportRequestError(error.userMessage, for: item.id) }
                    }
                }
                .buttonStyle(.borderedProminent)
                Button("Discard", role: .destructive) { Task { await store.discardFailure(item) } }
                    .buttonStyle(.bordered)
            }
        }
        .card()
    }
}
