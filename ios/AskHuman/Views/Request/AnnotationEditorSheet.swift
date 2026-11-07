import SwiftUI
import AHCore
import AHProtocol

/// The small sheet in which a note on a passage of the message is written or changed (docs/specs/message-annotations.md
/// D18): the passage on top, the text box under it, the keyboard up. It only draws what the form holds and hands the
/// note to the form; the screen decides what happens to the selection once the sheet is gone.
struct AnnotationEditorSheet: View {
    let edit: AnnotationEdit
    let composer: AnswerComposer
    let host: AnnotationHost

    @State private var text: String
    @State private var confirmDiscard = false
    @FocusState private var focused: Bool

    init(edit: AnnotationEdit, composer: AnswerComposer, host: AnnotationHost) {
        self.edit = edit
        self.composer = composer
        self.host = host
        // A note that is changed starts from what it says.
        if case .existing(let id) = edit.target {
            _text = State(initialValue: composer.annotation(id: id)?.note ?? "")
        } else {
            _text = State(initialValue: "")
        }
    }

    /// The number of the note that is changed; `nil` when a new one is written.
    private var existing: Int? {
        if case .existing(let id) = edit.target { id } else { nil }
    }

    private var quote: String {
        switch edit.target {
        case .new(let selection): selection.quote
        case .existing(let id): composer.annotation(id: id)?.quote ?? ""
        }
    }

    private var trimmed: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    // (Typed as keys on purpose: a choice between two literals could be taken for plain strings, which are not translated.)
    private var title: LocalizedStringKey { existing == nil ? "Add annotation" : "Edit annotation" }
    private var discardQuestion: LocalizedStringKey { existing == nil ? "Discard this annotation?" : "Discard your changes?" }

    /// Words that would be lost by leaving: what is written for a new note, what was changed in one that exists.
    /// A sheet in that state does not go away by a swipe or a tap on "Cancel" alone.
    private var hasUnsavedText: Bool {
        switch edit.target {
        case .new: !trimmed.isEmpty
        case .existing(let id): trimmed != (composer.annotation(id: id)?.note ?? "")
        }
    }

    /// Something is written, and the form still takes it (it may have been full when the sheet came up, or the note
    /// may have gone).
    private var canCommit: Bool {
        guard !trimmed.isEmpty else { return false }
        if let existing { return composer.annotation(id: existing) != nil }
        return composer.canAddAnnotation
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    PassageQuote(text: quote)
                    TextField("Write your question or comment…", text: $text, axis: .vertical)
                        .lineLimit(4...10)
                        .focused($focused)
                        .padding(14)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                        .accessibilityIdentifier("annotation-field")
                        .onChange(of: text) { _, new in
                            if new.count > Wire.maxNoteChars { text = String(new.prefix(Wire.maxNoteChars)) }
                        }
                    Text("Sent to the Agent with your answer.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 4)
                    if let existing {
                        Button("Delete annotation", role: .destructive) { delete(existing) }
                            .padding(.horizontal, 4)
                            .accessibilityIdentifier("annotation-delete")
                    }
                }
                .padding(.horizontal)
                .padding(.top, 8)
            }
            .scrollDismissesKeyboard(.never)
            .background(Color(.systemGroupedBackground))
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    CancelToolbarButton {
                        if hasUnsavedText { confirmDiscard = true } else { host.editing = nil }
                    }
                    .accessibilityIdentifier("annotation-cancel")
                }
                ToolbarItem(placement: .confirmationAction) {
                    if existing == nil {
                        ConfirmToolbarButton("Add", systemImage: "arrow.up", action: commit)
                            .disabled(!canCommit)
                            .accessibilityIdentifier("annotation-commit")
                    } else {
                        ConfirmToolbarButton("Save", systemImage: "checkmark", action: commit)
                            .disabled(!canCommit)
                            .accessibilityIdentifier("annotation-commit")
                    }
                }
            }
        }
        .presentationDetents([.medium, .large])
        .interactiveDismissDisabled(hasUnsavedText)
        .confirmationDialog(discardQuestion, isPresented: $confirmDiscard, titleVisibility: .visible) {
            Button("Discard", role: .destructive) { host.editing = nil }
            Button("Keep editing", role: .cancel) {}
        }
        .task { focused = true }
    }

    private func commit() {
        guard canCommit else { return }
        switch edit.target {
        case .new(let selection):
            composer.addAnnotation(start: selection.start, end: selection.end, quote: selection.quote, note: text)
        case .existing(let id):
            composer.setAnnotationNote(text, id: id)
        }
        host.editing = nil
    }

    private func delete(_ id: Int) {
        composer.removeAnnotation(id: id)
        host.editing = nil
    }
}

/// The passage a note is about: a purple bar (the colour of its highlight in the message) and the words.
struct PassageQuote: View {
    let text: String

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            RoundedRectangle(cornerRadius: 1.5)
                .fill(Color.purple)
                .frame(width: 3)
            Text(verbatim: text)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(3)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, 4)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("annotation-quote")
    }
}
