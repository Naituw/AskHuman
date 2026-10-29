import SwiftUI
import UIKit
import AHCore
import AHProtocol

/// The todos of one project: the queue (swipe to finish or delete, tap to edit, drag to reorder in edit
/// mode), the history, and a field to add the next one.
struct TodoProjectScreen: View {
    let target: OpenedTodoProject
    @Environment(AppStore.self) private var store
    @Environment(\.editMode) private var editMode
    @State private var editing: TodoItem?
    @State private var historyOpen = false
    /// The user has just added a todo: the list follows it to the end of the queue.
    @State private var revealEnd = false

    var body: some View {
        let todos = store.todos
        let list = todos.list
        ScrollViewReader { proxy in
            content(todos: todos, list: list)
                .onChange(of: list?.pending.count) { _, _ in
                    guard revealEnd, let last = store.todos.list?.pending.last else { return }
                    revealEnd = false
                    withAnimation { proxy.scrollTo(last.id, anchor: .bottom) }
                }
        }
    }

    private func content(todos: TodosStore, list: ProjectTodos?) -> some View {
        List {
            if !todos.canWrite {
                Section {
                    NoticeRow(
                        id: "todo-offline",
                        text: "\(todos.openedMacName ?? todos.accessMacName ?? String(localized: "The Mac")) is not reachable. This is the last state it reported.",
                        symbol: "desktopcomputer.trianglebadge.exclamationmark", tint: .secondary
                    )
                }
            }
            TodoProjectHeader(row: target.row, mac: todos.macs.count > 1 ? todos.openedMacName : nil)
            if let list {
                queue(list)
                history(list)
            } else {
                switch todos.listState {
                case .failed(let failure):
                    Section { TodoListFailure(failure: failure) }
                default:
                    SkeletonRows()
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollDismissesKeyboard(.interactively)
        // On the list itself: an identifier on the whole screen would cover the add bar's own.
        .accessibilityIdentifier("todo-project-screen")
        .navigationTitle(target.row.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .toolbar {
            if let list, list.pending.count > 1, todos.canWrite {
                ToolbarItem(placement: .topBarTrailing) { EditButton().accessibilityIdentifier("todo-reorder") }
            }
        }
        .actionBar {
            TodoAddBar(canAuto: list?.canAuto ?? false) { revealEnd = true }
        }
        .refreshable { await todos.refresh() }
        .overlay(alignment: .bottom) { TodoNoticeCapsule() }
        .sheet(item: $editing) { item in
            TodoEditSheet(item: item, canAuto: list?.canAuto ?? false)
        }
    }

    // MARK: queue

    @ViewBuilder
    private func queue(_ list: ProjectTodos) -> some View {
        let todos = store.todos
        Section {
            if list.pending.isEmpty {
                Text("Nothing to do here.")
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("todo-queue-empty")
            }
            ForEach(list.pending) { item in
                let sending = todos.isSending(item.id)
                Button {
                    if editMode?.wrappedValue != .active, !sending { editing = item }
                } label: {
                    TodoItemRow(item: item, sending: sending)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("todo-row-" + item.id)
                .swipeActions(edge: .leading, allowsFullSwipe: true) {
                    if !sending, todos.canWrite {
                        Button { todos.complete(item.id) } label: { Label("Done", systemImage: "checkmark.circle.fill") }
                            .tint(.green)
                    }
                }
                // Swiping the row open and tapping Delete is the confirmation, so there is no dialog after it; a swipe
                // all the way across does not delete (`allowsFullSwipe: false`).
                .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                    if !sending, todos.canWrite {
                        Button(role: .destructive) { todos.remove(item.id) } label: { Label("Delete", systemImage: "trash") }
                    }
                }
                .contextMenu {
                    Button { UIPasteboard.general.string = item.text } label: { Label("Copy", systemImage: "doc.on.doc") }
                }
                .moveDisabled(sending)
            }
            .onMove { source, destination in todos.move(from: source, to: destination) }
        } header: {
            HStack(spacing: 6) {
                Text("To do")
                Text("\(list.pendingTotal)").foregroundStyle(.secondary)
            }
            .font(.footnote.weight(.semibold))
            .textCase(nil)
        } footer: {
            if list.pendingTotal > UInt64(list.pending.count) {
                Text("Showing \(list.pending.count) of \(list.pendingTotal).")
            }
        }
    }

    // MARK: history

    @ViewBuilder
    private func history(_ list: ProjectTodos) -> some View {
        let todos = store.todos
        if list.historyTotal > 0 {
            Section {
                if historyOpen {
                    ForEach(list.history) { done in
                        TodoDoneRow(item: done)
                            .swipeActions(edge: .leading, allowsFullSwipe: true) {
                                if todos.canWrite {
                                    Button { todos.restore(done.id) } label: { Label("Restore", systemImage: "arrow.uturn.backward.circle.fill") }
                                        .tint(.blue)
                                }
                            }
                            .contextMenu {
                                Button { UIPasteboard.general.string = done.text } label: { Label("Copy", systemImage: "doc.on.doc") }
                            }
                            .accessibilityIdentifier("todo-done-" + done.id)
                    }
                    if list.historyTotal > UInt64(list.history.count) {
                        Text("Showing \(list.history.count) of \(list.historyTotal).")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            } header: {
                Button {
                    withAnimation(.snappy) { historyOpen.toggle() }
                } label: {
                    HStack(spacing: 6) {
                        Text("History")
                        Text("\(list.historyTotal)").foregroundStyle(.secondary)
                        Image(systemName: "chevron.right")
                            .font(.caption2.weight(.bold))
                            .foregroundStyle(.secondary)
                            .rotationEffect(.degrees(historyOpen ? 90 : 0))
                        Spacer(minLength: 0)
                    }
                    .font(.footnote.weight(.semibold))
                    .textCase(nil)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("todo-history-toggle")
            }
        }
    }
}

// MARK: pieces

/// Where the project lives: its repository, the Macs that have a folder for it.
private struct TodoProjectHeader: View {
    let row: TodoProjectRow
    /// The Mac that has the project, named when more than one Mac is paired.
    var mac: String?

    var body: some View {
        let lines = self.lines
        if !lines.isEmpty {
            Section {
                ForEach(lines.indices, id: \.self) { index in
                    Label {
                        lines[index].text.font(.subheadline).foregroundStyle(.secondary)
                    } icon: {
                        Image(systemName: lines[index].symbol).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    private struct Line {
        var symbol: String
        var text: Text
    }

    private var lines: [Line] {
        var out: [Line] = []
        if let hint = row.hint { out.append(Line(symbol: "folder", text: Text(verbatim: hint))) }
        if let repo = row.repo { out.append(Line(symbol: "link", text: Text(verbatim: repo))) }
        switch row.scope {
        case "cloudOnly":
            out.append(Line(symbol: "icloud", text: Text("Only in iCloud: no Mac has a folder for it")))
        case "localOnly":
            let text = mac.map { Text("Only on \($0), not synced") } ?? Text("Only on this Mac, not synced")
            out.append(Line(symbol: "desktopcomputer", text: text))
        default:
            if !row.macs.isEmpty {
                out.append(Line(symbol: "desktopcomputer", text: Text("On \(row.macs.joined(separator: ", "))")))
            }
        }
        return out
    }
}

private struct TodoItemRow: View {
    let item: TodoItem
    let sending: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 5) {
                Text(verbatim: item.text)
                    .font(.body)
                    .lineLimit(4)
                    .multilineTextAlignment(.leading)
                    .foregroundStyle(sending ? Color.secondary : Color.primary)
                if hasMeta { meta }
            }
            Spacer(minLength: 0)
            if sending {
                ProgressView().controlSize(.small)
                    .accessibilityLabel("Sending")
            }
        }
        .contentShape(Rectangle())
        .padding(.vertical, 2)
    }

    private var hasMeta: Bool {
        item.auto || item.att > 0 || item.attOn != nil || item.by != nil || item.agent != nil
    }

    private var meta: some View {
        FlowLayout(spacing: 6) {
            if item.auto { Chip(text: "Auto", systemImage: "bolt.fill", tint: .orange) }
            if item.att > 0 { Chip(verbatim: "\(item.att)", systemImage: "paperclip") }
            if let hint = item.attOn { Chip(verbatim: "\(hint.n) · \(hint.dev)", systemImage: "paperclip") }
            if let by = item.by { Chip(verbatim: by, systemImage: "desktopcomputer") }
            if let agent = item.agent { Chip(verbatim: TodoAgentName.label(agent), systemImage: "sparkles") }
        }
    }
}

private struct TodoDoneRow: View {
    let item: TodoDoneItem

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(verbatim: item.text)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(3)
                .multilineTextAlignment(.leading)
            HStack(spacing: 4) {
                if item.doneAtMs > 0 {
                    RelativeTimeText(date: Date(timeIntervalSince1970: Double(item.doneAtMs) / 1000))
                }
                if let device = item.doneOn { Text("· on \(device)") }
            }
            .font(.caption)
            .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 2)
    }
}

enum TodoAgentName {
    static func label(_ kind: String) -> String {
        switch kind {
        case "claude": "Claude Code"
        case "cursor": "Cursor"
        case "codex": "Codex"
        default: kind.prefix(1).uppercased() + kind.dropFirst()
        }
    }
}

private struct TodoListFailure: View {
    let failure: TodoFailure
    @Environment(AppStore.self) private var store

    var body: some View {
        let texts = TodoFailureTexts(failure, name: store.todos.openedMacName ?? store.todos.accessMacName)
        VStack(spacing: 8) {
            Image(systemName: texts.symbol).font(.title2).foregroundStyle(.secondary)
            Text(texts.title).font(.headline)
            texts.detail.font(.subheadline).foregroundStyle(.secondary).multilineTextAlignment(.center)
            if texts.canRetry {
                Button("Try again") { store.todos.reloadList() }.buttonStyle(.bordered)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 12)
    }
}

/// The few words a refused or lost write leaves behind, for a moment.
private struct TodoNoticeCapsule: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let todos = store.todos
        ZStack {
            if let item = todos.notice {
                Text(item.notice.message)
                    .font(.footnote.weight(.medium))
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .padding(.horizontal, 24)
                    .padding(.bottom, 90)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                    .accessibilityIdentifier("todo-notice")
                    .task(id: item.id) {
                        try? await Task.sleep(for: .seconds(4))
                        if !Task.isCancelled { todos.dismissNotice(item.id) }
                    }
            }
        }
        .animation(.snappy, value: todos.notice?.id)
    }
}

// MARK: add

/// The field at the bottom of a project: the next todo, added to the end of the queue.
private struct TodoAddBar: View {
    let canAuto: Bool
    /// The todo was taken (not yet confirmed by the Mac).
    let onAdded: () -> Void
    @Environment(AppStore.self) private var store
    @State private var text = ""
    @State private var auto = false
    @State private var problem: TodoTextError?
    @FocusState private var focused: Bool
    #if DEBUG
    @State private var stressReport: String?
    #endif

    var body: some View {
        let todos = store.todos
        if todos.canWrite {
            GlassGroup(spacing: 4) {
                VStack(spacing: 6) {
                    #if DEBUG
                    if let stressReport {
                        Color.clear.frame(width: 1, height: 1)
                            .accessibilityElement()
                            .accessibilityLabel(stressReport)
                            .accessibilityIdentifier("todo-add-stress-result")
                    }
                    #endif
                    HStack(alignment: .bottom, spacing: 8) {
                        if canAuto {
                            Button {
                                auto.toggle()
                            } label: {
                                autoLabel
                            }
                            .barRoundButton()
                            .accessibilityLabel("Auto-run")
                            .accessibilityValue(auto ? Text("On") : Text("Off"))
                            .accessibilityIdentifier("todo-add-auto")
                        }
                        TextField("New todo", text: $text, axis: .vertical)
                            .lineLimit(1...5)
                            .focused($focused)
                            .accessibilityIdentifier("todo-add-field")
                            .barFieldBox(focus: $focused)
                        BarSendButton(label: "Add todo", identifier: "todo-add-send", action: send)
                            .disabled(!valid)
                    }
                    if trimmedCount > Wire.todoTextMaxChars - 500 {
                        Text("\(trimmedCount) / \(Wire.todoTextMaxChars)")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(trimmedCount > Wire.todoTextMaxChars ? Color.red : Color.secondary)
                            .barStatusChip(compact: true)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    }
                }
            }
            .padding(.horizontal, BottomBar.floats ? 16 : 12)
            .padding(.vertical, 8)
            .classicBarBacking(hairline: false)
            #if DEBUG
            .task(id: focused) { await stressWhenAsked() }
            #endif
        } else {
            Text("Todos can be added when the Mac is reachable.")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(BottomBar.floats ? .center : .leading)
                .barStatusChip(compact: true)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 16)
                .padding(.vertical, BottomBar.floats ? 8 : 12)
                .classicBarBacking(hairline: false)
                .accessibilityIdentifier("todo-add-unavailable")
        }
    }

    /// The auto-run toggle: a bolt on a glass circle from iOS 26, the bolt in a circle before. Orange while it
    /// is on, as it always was.
    @ViewBuilder
    private var autoLabel: some View {
        if BottomBar.floats {
            Image(systemName: auto ? "bolt.fill" : "bolt")
                .barRoundGlyph()
                .foregroundStyle(auto ? Color.orange : Color.secondary)
        } else {
            Image(systemName: auto ? "bolt.circle.fill" : "bolt.circle")
                .font(.system(size: 30))
                .foregroundStyle(auto ? Color.orange : Color.secondary)
                .frame(width: 34, height: 34)
        }
    }

    private var trimmedCount: Int { text.trimmingCharacters(in: .whitespacesAndNewlines).count }

    private var valid: Bool {
        if case .success = TodoText.validate(text) { return true }
        return false
    }

    private func send() {
        guard store.todos.add(text, auto: canAuto && auto) == nil else { return }
        // This bar's field is the only text input on the page, so it is the one that has the keyboard.
        FocusedTextInput.clear()
        text = ""
        auto = false
        onAdded()
    }

    #if DEBUG
    /// UI-test hook (`-AHTodoAddStress YES`): once the field has the keyboard, types into it and sends in the same
    /// turn, again and again, and reports how many times the field kept what was typed.
    private func stressWhenAsked() async {
        guard focused, stressReport == nil, UserDefaults.standard.bool(forKey: "AHTodoAddStress") else { return }
        let rounds = 30
        var kept = 0
        for round in 0..<rounds {
            guard FocusedTextInput.type("todo \(round)", marked: round % 2 == 1) else {
                stressReport = "no input after \(round) of \(rounds)"
                return
            }
            send()
            try? await Task.sleep(for: .milliseconds(450))
            if !(FocusedTextInput.text ?? "").isEmpty { kept += 1 }
        }
        stressReport = "kept \(kept) of \(rounds)"
    }
    #endif
}

/// The text input that has the keyboard, reached through the responder chain.
///
/// Emptying a `TextField` by setting its binding is not reliable when that happens in the same update as the
/// last keystroke: SwiftUI pushes a binding into the field only when it differs from what SwiftUI last saw there,
/// and a tap on Send that follows the typing closely enough sets the binding back to exactly that, so the field
/// keeps what was typed while the state says it is empty. The field's own text is not subject to that comparison.
/// Clearing it makes UIKit report the change to SwiftUI, and the keyboard keeps its focus.
@MainActor
enum FocusedTextInput {
    private static var captured: UIResponder?

    /// Ends any composition in progress and removes all the text. `false` when no text input has the keyboard.
    @discardableResult
    static func clear() -> Bool {
        guard let input = current() else { return false }
        input.unmarkText()
        if let all = input.textRange(from: input.beginningOfDocument, to: input.endOfDocument) {
            input.replace(all, withText: "")
        }
        return true
    }

    static func capture(_ responder: UIResponder) { captured = responder }

    private static func current() -> UITextInput? {
        captured = nil
        // Sent to the first responder, whatever it is; it records itself in `capture`.
        UIApplication.shared.sendAction(#selector(UIResponder.ahCaptureAsFocusedTextInput), to: nil, from: nil, for: nil)
        defer { captured = nil }
        return captured as? UITextInput
    }

    #if DEBUG
    /// What the input holds right now, composition included. `nil` when none has the keyboard.
    static var text: String? {
        guard let input = current(), let all = input.textRange(from: input.beginningOfDocument, to: input.endOfDocument) else { return nil }
        return input.text(in: all)
    }

    /// Types into the input the way the keyboard does: as a composition (`marked`) or as committed text.
    @discardableResult
    static func type(_ string: String, marked: Bool) -> Bool {
        guard let input = current() else { return false }
        if marked {
            input.setMarkedText(string, selectedRange: NSRange(location: string.utf16.count, length: 0))
        } else {
            input.insertText(string)
        }
        return true
    }
    #endif
}

extension UIResponder {
    @MainActor @objc fileprivate func ahCaptureAsFocusedTextInput() { FocusedTextInput.capture(self) }
}
