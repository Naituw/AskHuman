import Foundation
import Observation
import AHProtocol
import AHStore

/// The message being written to an Agent: text and a few attachments. Attachments are kept
/// in a temporary folder until the message is sent or the form is discarded.
@MainActor
@Observable
public final class InterjectComposer {
    /// The longest message the Mac accepts.
    public static let maxChars = 20_000

    public let key: String
    public var text = ""
    public private(set) var attachments: [DraftAttachment] = []
    public var isSending = false
    private var attachmentPreparations: Set<UUID> = []
    public var isPreparingAttachments: Bool { !attachmentPreparations.isEmpty }

    public func setAttachmentPreparation(_ id: UUID, active: Bool) {
        if active { attachmentPreparations.insert(id) } else { attachmentPreparations.remove(id) }
    }
    public var uploadProgress: Double?
    /// Set while a send may or may not have reached the Mac: sending again reuses it so the Mac
    /// does not deliver the message twice.
    @ObservationIgnored public var pendingMessageId: String?
    public var needsDeliveryReview = false

    /// Called only after the user checked the original chat and chooses a fresh submission.
    public func confirmDeliveryReviewed() {
        pendingMessageId = nil
        needsDeliveryReview = false
    }

    @ObservationIgnored private let directory: URL

    public init(key: String) {
        self.key = key
        directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("ah-interject", isDirectory: true)
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
    }

    deinit {
        try? FileManager.default.removeItem(at: directory)
    }

    public var trimmedText: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }
    public var overLimit: Bool { trimmedText.count > Self.maxChars }
    public var hasContent: Bool { !trimmedText.isEmpty || !attachments.isEmpty }
    public var canSend: Bool { hasContent && !overLimit && !isSending && !isPreparingAttachments }
    public var canAttachMore: Bool { attachments.count < Wire.maxReplyAttachments }

    // MARK: attachments

    @discardableResult
    public func addAttachment(name: String, mime: String, data: Data, isImage: Bool) throws -> DraftAttachment {
        guard data.count <= Wire.maxAssetBytes else { throw BackendError.tooLarge }
        guard canAttachMore else { throw BackendError.invalid("At most \(Wire.maxReplyAttachments) attachments per message.") }
        let id = Bytes.newId()
        let localName = id + "-" + Self.safeFileName(name)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try data.write(to: directory.appendingPathComponent(localName), options: .atomic)
        let attachment = DraftAttachment(id: id, name: name, mime: mime, size: data.count, isImage: isImage, localName: localName)
        attachments.append(attachment)
        return attachment
    }

    public func removeAttachment(id: String) {
        guard let index = attachments.firstIndex(where: { $0.id == id }) else { return }
        let removed = attachments.remove(at: index)
        try? FileManager.default.removeItem(at: url(of: removed))
    }

    public func url(of attachment: DraftAttachment) -> URL {
        directory.appendingPathComponent(attachment.localName)
    }

    /// The files as the backend uploads them.
    public func sources() throws -> [UploadSource] {
        try attachments.map { attachment in
            UploadSource(name: attachment.name, mime: attachment.mime, data: try Data(contentsOf: url(of: attachment)), isImage: attachment.isImage)
        }
    }

    // MARK: lifecycle

    /// The message went out: the form is empty again.
    public func reset() {
        text = ""
        attachments = []
        pendingMessageId = nil
        needsDeliveryReview = false
        uploadProgress = nil
        try? FileManager.default.removeItem(at: directory)
    }

    private static func safeFileName(_ name: String) -> String {
        let cleaned = name.map { $0.isLetter || $0.isNumber || $0 == "." || $0 == "-" || $0 == "_" ? $0 : "_" }
        return String(String(cleaned).prefix(80))
    }
}

/// Continuation has its own identity so it never overwrites an unsent free-form draft.
@MainActor
@Observable
public final class AgentContinuation {
    public var isSending = false
    public var needsDeliveryReview = false
    public internal(set) var messageId: String?
    public internal(set) var resultId: String?
    public internal(set) var submittedResultId: String?
    public init() {}
    public func confirmDeliveryReviewed() {
        messageId = nil
        needsDeliveryReview = false
    }
}
