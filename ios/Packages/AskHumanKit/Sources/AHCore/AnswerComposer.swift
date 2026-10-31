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

    public init(key: String, ask: AskView, drafts: DraftStore, debounce: Duration = .milliseconds(300)) {
        self.key = key
        self.ask = ask
        self.drafts = drafts
        self.debounce = debounce
        let stored = drafts.draft(key: key)?.answers ?? []
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
    }

    // MARK: rules

    public var selectOnly: Bool { ask.selectOnly }
    public var single: Bool { ask.single }
    public var questionCount: Int { ask.questions.count }

    /// A question counts as answered when something was selected, written or attached.
    public func isAnswered(_ question: Int) -> Bool {
        guard answers.indices.contains(question) else { return false }
        return !answers[question].isEmpty
    }

    public var answeredCount: Int { answers.indices.filter(isAnswered).count }

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

    public var hasContent: Bool { answers.contains { !$0.isEmpty } }

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
        answers[question].selected = selected
        changed()
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

    // MARK: sending

    /// The wire answers: one item per answered question, with the upload ids of its attachments.
    public func items(uploads: [String: String]) -> [AnswerItem] {
        answers.indices.compactMap { index in
            guard isAnswered(index) else { return nil }
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
        let snapshot = Draft(answers: answers)
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
        drafts.save(Draft(answers: answers), key: key)
    }

    private static func safeFileName(_ name: String) -> String {
        let cleaned = name.map { "/\\:".contains($0) ? "_" : $0 }
        let text = String(cleaned).trimmingCharacters(in: .whitespaces)
        return text.isEmpty ? "file" : String(text.suffix(80))
    }
}
