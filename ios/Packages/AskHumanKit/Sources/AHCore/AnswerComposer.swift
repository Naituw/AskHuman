import Foundation
import Observation
import AHProtocol
import AHStore

/// The state of the answer form of one question request: selections, free text and reply
/// attachments of every question, with the rules of the Mac popup (`selectOnly`, `single`) and
/// automatic draft saving.
@MainActor
@Observable
public final class AnswerComposer {
    public let key: String
    public let ask: AskView
    public private(set) var answers: [DraftAnswer]
    /// Ids of the project todos picked to go with the answer (docs/specs/special-replies.md D28): the Mac puts
    /// their text into the answer and takes them off the queue. In the order the picker lists them.
    public private(set) var todoIds: [String]
    /// Notes written on passages of the message (docs/specs/message-annotations.md), in reading order: the notes
    /// whose passage the page can show by their position in the message, then the ones it could not find (N3).
    /// They are not an answer: they count as content of the form (N8), but answer no question and change nothing
    /// about what Send requires.
    public private(set) var annotations: [DraftAnnotation]
    /// Notes the message page could not find in the message as it is drawn now: the text changed since the note
    /// was written. Not saved, the page finds out again each time it draws.
    public private(set) var unlocatedAnnotations: Set<Int> = []
    /// What the user chose from the bar menu to go with the answer (away mode on, stop using AskHuman): armed
    /// until the answer is sent or the user takes it back. Not part of the draft: it is a decision of the moment,
    /// and a draft that comes back later should not turn it on by itself.
    public var directive: AnswerDirectiveChoice?
    /// Upload progress while sending (0…1), `nil` otherwise.
    public var uploadProgress: Double?
    public var isSending = false
    private var attachmentPreparations: Set<UUID> = []
    private var preparationStates: [Int: AttachmentPreparationState] = [:]
    public var isPreparingAttachments: Bool { !attachmentPreparations.isEmpty }

    public func attachmentPreparation(for question: Int) -> AttachmentPreparationState {
        if let state = preparationStates[question] { return state }
        let state = AttachmentPreparationState()
        preparationStates[question] = state
        return state
    }

    public func setAttachmentPreparation(_ id: UUID, active: Bool) {
        if active { attachmentPreparations.insert(id) } else { attachmentPreparations.remove(id) }
    }

    @ObservationIgnored private let drafts: DraftStore
    @ObservationIgnored private var saveTask: Task<Void, Never>?
    @ObservationIgnored private let debounce: Duration
    /// Never goes back down while the form lives, so that a late report of the page about a note that is gone
    /// cannot land on a new note that took its number.
    @ObservationIgnored private var nextAnnotationId: Int

    public init(key: String, ask: AskView, drafts: DraftStore, debounce: Duration = .milliseconds(300)) {
        self.key = key
        self.ask = ask
        self.drafts = drafts
        self.debounce = debounce
        let draft = drafts.draft(key: key)
        let stored = draft?.answers ?? []
        // A select-only request takes no todos, and a What's next answer is a todo or an option, not both.
        todoIds = ask.selectOnly ? [] : Array((draft?.todoIds ?? []).prefix(ask.whatsNext ? 1 : Wire.maxPickedTodos))
        // Notes belong to a request that offered them: the Mac would drop them from any other.
        let restored = ask.annotatable ? Self.restoredAnnotations(draft?.annotations ?? []) : []
        annotations = restored
        nextAnnotationId = (restored.map(\.id).max() ?? 0) + 1
        answers = ask.questions.indices.map { index in
            guard index < stored.count else { return DraftAnswer() }
            var answer = stored[index]
            // The content may have changed since the draft was written: drop what no longer exists.
            let optionCount = ask.questions[index].options.count
            answer.selected = answer.selected.filter { $0 >= 0 && $0 < optionCount }
            if ask.single, answer.selected.count > 1 { answer.selected = Array(answer.selected.prefix(1)) }
            if ask.selectOnly {
                answer.text = ""
                answer.attachments = []
            }
            return answer
        }
        if ask.whatsNext, !todoIds.isEmpty, !answers.isEmpty { answers[0].selected = [] }
    }

    // MARK: rules

    public var selectOnly: Bool { ask.selectOnly }
    public var single: Bool { ask.single }
    public var questionCount: Int { ask.questions.count }

    /// A question counts as answered when something was selected, written or attached. The picked todos go
    /// into the last question's answer, so they answer it.
    public func isAnswered(_ question: Int) -> Bool {
        guard answers.indices.contains(question) else { return false }
        if question == questionCount - 1, !todoIds.isEmpty { return true }
        return !answers[question].isEmpty
    }

    public var answeredCount: Int { answers.indices.filter(isAnswered).count }

    /// Whether a question is a bare input: the form takes free text and the question offers no options,
    /// so typing is all there is to do in it. (With options, the text is a note beside a choice.)
    public func isTextOnly(_ question: Int) -> Bool {
        guard !selectOnly, ask.questions.indices.contains(question) else { return false }
        return ask.questions[question].options.isEmpty
    }

    /// Select-only requests need a selection for every question; others may be sent as they are.
    public var canSend: Bool {
        if isSending || isPreparingAttachments { return false }
        if selectOnly { return answers.allSatisfy { !$0.selected.isEmpty } }
        return true
    }

    /// Why the send button is disabled, if it is.
    public var blockedReason: BlockedReason? {
        if selectOnly, !answers.allSatisfy({ !$0.selected.isEmpty }) { return .needsSelection }
        return nil
    }

    public enum BlockedReason: Equatable, Sendable { case needsSelection }

    // MARK: reading progress

    /// Questions whose card bottom has been on screen. As in the Mac popup (spec
    /// `multi-question-vertical.md` V1/V2) the form of several questions cannot be sent before the
    /// last question was seen, so that a stray tap cannot send what was never read.
    ///
    /// This only guards "Send". Where "Next" leads does not depend on it: that follows the position
    /// on screen (`QuestionNavigator`), so a question that was seen but not answered is not skipped.
    public private(set) var seen: Set<Int> = []

    public func markSeen(_ question: Int) {
        // Observation notifies on every write, even of a value already there: scrolling must not.
        guard questionCount > 1, (0..<questionCount).contains(question), !seen.contains(question) else { return }
        seen.insert(question)
    }

    /// A question counts as read once it was seen, or answered (which takes a look at it).
    public func hasRead(_ question: Int) -> Bool {
        seen.contains(question) || isAnswered(question)
    }

    /// Whether the last question was read, which is what lets a form of several questions be sent.
    /// A single question is read the moment the form opens. Once true it stays true, also when the
    /// person scrolls back up.
    public var hasReachedEnd: Bool {
        questionCount <= 1 || hasRead(questionCount - 1)
    }

    /// In a select-only form, the first question that still has no selection: what "Send" is waiting
    /// for. `nil` when the form is complete or takes free text.
    public var firstWithoutSelection: Int? {
        guard selectOnly else { return nil }
        return answers.indices.first { answers[$0].selected.isEmpty }
    }

    /// Whether the form holds anything the user wrote or chose, notes included (they are kept in the draft, N8).
    /// Notes alone do not make a question answered, though: see `isAnswered`.
    public var hasContent: Bool { !todoIds.isEmpty || !annotations.isEmpty || answers.contains { !$0.isEmpty } }

    /// Attachments of the whole form; the Mac accepts a handful per request.
    public var attachmentCount: Int { answers.reduce(0) { $0 + $1.attachments.count } }
    public var canAttachMore: Bool { !selectOnly && attachmentCount < Wire.maxReplyAttachments }

    // MARK: editing

    public func isSelected(question: Int, option: Int) -> Bool {
        guard answers.indices.contains(question) else { return false }
        return answers[question].selected.contains(option)
    }

    public func toggle(question: Int, option: Int) {
        guard answers.indices.contains(question), ask.questions[question].options.indices.contains(option) else { return }
        var selected = answers[question].selected
        if let at = selected.firstIndex(of: option) {
            selected.remove(at: at)
        } else if single {
            selected = [option]
        } else {
            selected.append(option)
            selected.sort()
        }
        // What's next: the next task is one choice, a todo of the queue or an option.
        if ask.whatsNext, !selected.isEmpty { todoIds = [] }
        answers[question].selected = selected
        changed()
    }

    // MARK: todos

    public func isTodoPicked(_ id: String) -> Bool { todoIds.contains(id) }

    /// Replaces the picked todos; the picker hands them over in the order of its list. In What's next a todo
    /// is the next task, so picking one lets go of the option that was chosen.
    public func setTodoIds(_ ids: [String]) {
        guard !selectOnly else { return }
        let ids = Array(ids.prefix(ask.whatsNext ? 1 : Wire.maxPickedTodos))
        guard ids != todoIds else { return }
        todoIds = ids
        if ask.whatsNext, !ids.isEmpty, !answers.isEmpty { answers[0].selected = [] }
        changed()
    }

    /// Lets go of the picked todos that are no longer in the queue (somebody finished or removed them).
    public func keepTodos(in available: Set<String>) {
        setTodoIds(todoIds.filter(available.contains))
    }

    /// What's next with the whole queue listed: the options the question carries for queue entries are not
    /// shown any more, so a choice of one of them is let go of (it would be a hidden choice otherwise).
    public func dropTodoOptionSelections() {
        guard ask.whatsNext else { return }
        var dropped = false
        for question in answers.indices {
            let options = ask.questions[question].options
            let kept = answers[question].selected.filter { options.indices.contains($0) && !options[$0].todo }
            if kept != answers[question].selected {
                answers[question].selected = kept
                dropped = true
            }
        }
        if dropped { changed() }
    }

    public func setText(_ text: String, question: Int) {
        guard answers.indices.contains(question), !selectOnly, answers[question].text != text else { return }
        answers[question].text = String(text.prefix(Wire.maxAnswerTextChars))
        changed()
    }

    /// Stores the bytes of a new attachment next to the draft and lists it under the question.
    @discardableResult
    public func addAttachment(name: String, mime: String, data: Data, isImage: Bool, question: Int) throws -> DraftAttachment {
        guard answers.indices.contains(question), !selectOnly else { throw BackendError.invalid("This question takes no attachments.") }
        guard data.count <= Wire.maxAssetBytes else { throw BackendError.tooLarge }
        guard attachmentCount < Wire.maxReplyAttachments else {
            throw BackendError.invalid("At most \(Wire.maxReplyAttachments) attachments per reply.")
        }
        let id = Bytes.newId()
        let localName = id + "-" + Self.safeFileName(name)
        let directory = drafts.filesDirectory(key: key)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try data.write(to: directory.appendingPathComponent(localName), options: .atomic)
        let attachment = DraftAttachment(id: id, name: name, mime: mime, size: data.count, isImage: isImage, localName: localName)
        answers[question].attachments.append(attachment)
        changed()
        return attachment
    }

    public func removeAttachment(id: String, question: Int) {
        guard answers.indices.contains(question),
              let at = answers[question].attachments.firstIndex(where: { $0.id == id }) else { return }
        let removed = answers[question].attachments.remove(at: at)
        try? FileManager.default.removeItem(at: url(of: removed))
        changed()
    }

    public func url(of attachment: DraftAttachment) -> URL {
        drafts.filesDirectory(key: key).appendingPathComponent(attachment.localName)
    }

    // MARK: notes on the message (docs/specs/message-annotations.md)

    /// Whether the form is full of notes (`Wire.maxAnnotations`).
    public var annotationLimitReached: Bool { annotations.count >= Wire.maxAnnotations }

    /// Whether one more note may be written: the Mac offered notes for this request, and there is room.
    /// Whether the phone can show the message for selecting is for the screen to know.
    public var canAddAnnotation: Bool { ask.annotatable && !annotationLimitReached }

    public func annotation(id: Int) -> DraftAnnotation? { annotations.first { $0.id == id } }

    /// Whether the page found the note's passage when it last drew the message.
    public func isLocated(_ id: Int) -> Bool { !unlocatedAnnotations.contains(id) }

    /// Keeps a note about the passage `start..<end` (offsets into the text of the message page, as the page
    /// reported them), reading order restored. `quote` is the passage as seen. Nothing is added, and `nil` comes
    /// back, when notes are not offered, the form is full, or the passage or the note is empty.
    @discardableResult
    public func addAnnotation(start: Int, end: Int, quote: String, note: String) -> DraftAnnotation? {
        let quote = Self.cleanQuote(quote)
        let note = Self.cleanNote(note)
        guard canAddAnnotation, start >= 0, end > start, !quote.isEmpty, !note.isEmpty else { return nil }
        let added = DraftAnnotation(id: nextAnnotationId, start: start, end: end, quote: quote, note: note)
        nextAnnotationId += 1
        annotations.append(added)
        sortAnnotations()
        changed()
        return added
    }

    /// Replaces what a note says. False when there is no such note or the new text is empty (the note then stays as it
    /// was; taking a note away is `removeAnnotation`).
    @discardableResult
    public func setAnnotationNote(_ note: String, id: Int) -> Bool {
        guard let index = annotations.firstIndex(where: { $0.id == id }) else { return false }
        let note = Self.cleanNote(note)
        guard !note.isEmpty else { return false }
        if annotations[index].note != note {
            annotations[index].note = note
            changed()
        }
        return true
    }

    public func removeAnnotation(id: Int) {
        guard let index = annotations.firstIndex(where: { $0.id == id }) else { return }
        annotations.remove(at: index)
        if unlocatedAnnotations.contains(id) { unlocatedAnnotations.remove(id) }
        changed()
    }

    /// What the message page found for one note when it drew the message.
    public struct AnnotationPosition: Equatable, Sendable {
        public var id: Int
        public var start: Int
        public var end: Int
        /// False when neither the stored offsets nor a search for the quote found the passage.
        public var located: Bool

        public init(id: Int, start: Int, end: Int, located: Bool) {
            self.id = id
            self.start = start
            self.end = end
            self.located = located
        }
    }

    /// Takes what the page reports after drawing the message: the offsets a note really sits at now (kept in the
    /// draft, so that the next drawing starts from them) and which notes it could not find. Reading order is
    /// restored, the notes without a place last (N3). Reports about notes that are gone are ignored; nothing is
    /// written, or saved, when nothing changed, because the page reports after every drawing.
    public func applyAnnotationPositions(_ reports: [AnnotationPosition]) {
        var moved = false
        var lost = unlocatedAnnotations
        for report in reports {
            guard let index = annotations.firstIndex(where: { $0.id == report.id }) else { continue }
            guard report.located else {
                lost.insert(report.id)
                continue
            }
            lost.remove(report.id)
            guard report.start >= 0, report.end > report.start,
                  annotations[index].start != report.start || annotations[index].end != report.end else { continue }
            annotations[index].start = report.start
            annotations[index].end = report.end
            moved = true
        }
        if lost != unlocatedAnnotations { unlocatedAnnotations = lost }
        if sortAnnotations() || moved { changed() }
    }

    /// The notes as the wire carries them, in reading order.
    public var annotationItems: [AnnotationItem] {
        annotations.map { AnnotationItem(quote: $0.quote, note: $0.note) }
    }

    /// Notes with a place by position, then the ones without (stable: notes at one position keep their order).
    /// True when the order changed.
    @discardableResult
    private func sortAnnotations() -> Bool {
        let ordered = annotations.enumerated().sorted { left, right in
            let leftLost = unlocatedAnnotations.contains(left.element.id)
            let rightLost = unlocatedAnnotations.contains(right.element.id)
            if leftLost != rightLost { return rightLost }
            if !leftLost, left.element.start != right.element.start { return left.element.start < right.element.start }
            return left.offset < right.offset
        }.map(\.element)
        guard ordered != annotations else { return false }
        annotations = ordered
        return true
    }

    /// A draft written for a request is taken back as it is, minus what cannot be right: no more than the limit,
    /// no empty or repeated note, no negative offsets (the page finds a passage by its quote when the offsets
    /// are off).
    private static func restoredAnnotations(_ stored: [DraftAnnotation]) -> [DraftAnnotation] {
        var seen: Set<Int> = []
        var kept: [DraftAnnotation] = []
        for var note in stored {
            note.quote = cleanQuote(note.quote)
            note.note = cleanNote(note.note)
            guard !note.quote.isEmpty, !note.note.isEmpty, seen.insert(note.id).inserted else { continue }
            note.start = max(0, note.start)
            note.end = max(note.start + 1, note.end)
            kept.append(note)
            if kept.count == Wire.maxAnnotations { break }
        }
        return kept
    }

    /// What the user wrote, without the blanks around it, cut to the limit the Mac keeps.
    private static func cleanNote(_ text: String) -> String {
        String(text.trimmingCharacters(in: .whitespacesAndNewlines).prefix(Wire.maxNoteChars))
    }

    /// A passage on one line (runs of blanks and line breaks become one space), cut in the middle with `…` where it
    /// is longer than the Mac keeps. The page already hands passages over like this; this keeps a wrong one from
    /// reaching the wire.
    private static func cleanQuote(_ text: String) -> String {
        let folded = text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let scalars = Array(folded.unicodeScalars)
        guard scalars.count > Wire.maxQuoteChars else { return folded }
        let head = (Wire.maxQuoteChars - 1) / 2
        let tail = Wire.maxQuoteChars - 1 - head
        var cut = String.UnicodeScalarView()
        cut.append(contentsOf: scalars.prefix(head))
        cut.append("…")
        cut.append(contentsOf: scalars.suffix(tail))
        return String(cut)
    }

    // MARK: quick replies

    /// The quick replies the question carries, in the order the menu lists them (docs/specs/special-replies.md D27):
    /// the Mac's own list for this request. None for What's next and a strict choice, where a sentence is no answer.
    public var quickReplies: [String] {
        guard !selectOnly, !ask.whatsNext else { return [] }
        return ask.quickReplies
    }

    /// Whether a quick reply would be this question's answer (D18): the question has nothing in it yet. Picked
    /// project todos go with the last question, so they answer it.
    public func quickReplyFills(_ question: Int) -> Bool {
        answers.indices.contains(question) && !isAnswered(question)
    }

    /// How many questions a click on a quick reply would answer; the menu greys the replies out at 0.
    public var quickReplyTargets: Int {
        quickReplies.isEmpty ? 0 : answers.indices.filter(quickReplyFills).count
    }

    // MARK: sending

    /// The wire answers: one item per answered question, with the upload ids of its attachments. With a quick reply
    /// (D18) every question that has no answer yet gets it as its text; the answered ones stay as they are.
    public func items(uploads: [String: String], fillingWith quickReply: String? = nil) -> [AnswerItem] {
        let reply = quickReply?.trimmingCharacters(in: .whitespacesAndNewlines)
        return answers.indices.compactMap { index in
            // The picked todos travel beside the items (`todoIds`), not as an item of their own.
            guard !answers[index].isEmpty else {
                guard let reply, !reply.isEmpty, quickReplies.contains(quickReply ?? ""), quickReplyFills(index) else { return nil }
                return AnswerItem(q: index, sel: [], text: reply, images: [], files: [])
            }
            let answer = answers[index]
            let text = answer.text.trimmingCharacters(in: .whitespacesAndNewlines)
            var images: [String] = []
            var files: [String] = []
            for attachment in answer.attachments {
                guard let uploadId = uploads[attachment.id] else { continue }
                if attachment.isImage { images.append(uploadId) } else { files.append(uploadId) }
            }
            return AnswerItem(q: index, sel: answer.selected.sorted(), text: text.isEmpty ? nil : text, images: images, files: files)
        }
    }

    /// Forgets the draft (the request is finished).
    public func discard() {
        saveTask?.cancel()
        for state in preparationStates.values { state.cancel() }
        drafts.remove(key: key)
    }

    // MARK: saving

    private func changed() {
        saveTask?.cancel()
        let snapshot = Draft(answers: answers, todoIds: todoIds, annotations: annotations)
        let drafts = drafts
        let key = key
        let debounce = debounce
        saveTask = Task {
            try? await Task.sleep(for: debounce)
            guard !Task.isCancelled else { return }
            drafts.save(snapshot, key: key)
        }
    }

    /// Writes the draft right now (the form is leaving the screen).
    public func flush() {
        saveTask?.cancel()
        drafts.save(Draft(answers: answers, todoIds: todoIds, annotations: annotations), key: key)
    }

    private static func safeFileName(_ name: String) -> String {
        let cleaned = name.map { "/\\:".contains($0) ? "_" : $0 }
        let text = String(cleaned).trimmingCharacters(in: .whitespaces)
        return text.isEmpty ? "file" : String(text.suffix(80))
    }
}
