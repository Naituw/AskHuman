import Foundation
import AHProtocol
import AHStore

/// What the Markdown page and the app tell each other about the notes written on the message
/// (docs/specs/message-annotations.md; the page's side is `notes.js` in the app's `WebMarkdown` folder).
///
/// The page speaks in script messages whose bodies arrive from JavaScript as plain values, and it takes script
/// calls that the app writes as text. Both are kept here, apart from the web view, so that what they accept and
/// what they write can be tested.
public enum NotesPage {
    /// A passage of the message the page reports as selected.
    public struct Selection: Equatable, Sendable {
        public var quote: String
        /// Offsets into the text of the message as the page counts it (`start..<end`).
        public var start: Int
        public var end: Int
        /// The note whose passage this one touches: the menu then offers to edit it.
        public var overlapId: Int?

        public init(quote: String, start: Int, end: Int, overlapId: Int? = nil) {
            self.quote = quote
            self.start = start
            self.end = end
            self.overlapId = overlapId
        }
    }

    /// A note as the page needs it to draw it: where it sat and what it quoted.
    public struct Note: Equatable, Sendable, Encodable {
        public var id: Int
        public var start: Int
        public var end: Int
        public var quote: String

        public init(id: Int, start: Int, end: Int, quote: String) {
            self.id = id
            self.start = start
            self.end = end
            self.quote = quote
        }
    }

    /// The passage a note is being written on, which the page outlines meanwhile.
    public struct Anchor: Equatable, Sendable, Encodable {
        public var start: Int
        public var end: Int

        public init(start: Int, end: Int) {
            self.start = start
            self.end = end
        }
    }

    /// What the selection menu offers for the passage that is selected.
    public enum Offer: Equatable, Sendable {
        /// A note on this passage.
        case add(Selection)
        /// The passage is (partly) one of a note: change that one.
        case edit(id: Int)
        /// The form is full of notes: nothing more is taken, and the menu says so.
        case full
    }

    // MARK: what the page says

    /// The selection a `selection` message carries, `nil` for none (the page sends `null`) or for a body that is not one.
    public static func selection(from body: Any?) -> Selection? {
        guard let fields = body as? [String: Any],
              let quote = fields["quote"] as? String, !quote.isEmpty,
              let start = integer(fields["start"]), let end = integer(fields["end"]),
              start >= 0, end > start else { return nil }
        return Selection(quote: quote, start: start, end: end, overlapId: integer(fields["overlapId"]))
    }

    /// The id of the note a `noteTap` message names.
    public static func noteID(from body: Any?) -> Int? { integer(body) }

    /// What a `notes` message says about where each note sits; entries that are not well formed are left out.
    public static func positions(from body: Any?) -> [AnswerComposer.AnnotationPosition] {
        guard let list = body as? [Any] else { return [] }
        return list.compactMap { entry in
            guard let fields = entry as? [String: Any], let id = integer(fields["id"]),
                  let start = integer(fields["start"]), let end = integer(fields["end"]),
                  let located = flag(fields["located"]) else { return nil }
            return AnswerComposer.AnnotationPosition(id: id, start: start, end: end, located: located)
        }
    }

    /// A whole number out of a script message (the page's numbers arrive as `NSNumber`).
    private static func integer(_ value: Any?) -> Int? {
        guard let number = value as? NSNumber, !isBoolean(number) else { return nil }
        let double = number.doubleValue
        // Offsets and ids are small: anything else is not what the page would send.
        guard double.isFinite, double == double.rounded(), abs(double) < 1_000_000_000 else { return nil }
        return Int(double)
    }

    private static func flag(_ value: Any?) -> Bool? {
        guard let number = value as? NSNumber, isBoolean(number) else { return nil }
        return number.boolValue
    }

    private static func isBoolean(_ number: NSNumber) -> Bool { CFGetTypeID(number) == CFBooleanGetTypeID() }

    // MARK: what the app tells the page

    /// `AH.setNotes(…)`: the notes the page draws, in reading order.
    public static func setNotes(_ notes: [Note]) -> String { "AH.setNotes(\(json(notes, fallback: "[]")))" }

    /// `AH.setPending(…)`: the passage a note is being written on gets an outline; `nil` takes it away.
    public static func setPending(_ anchor: Anchor?) -> String {
        guard let anchor else { return "AH.setPending(null)" }
        return "AH.setPending(\(json(anchor, fallback: "null")))"
    }

    public static func locate(_ id: Int) -> String { "AH.locate(\(id))" }

    public static func flash(_ id: Int) -> String { "AH.flash(\(id))" }

    public static let clearSelection = "AH.clearSelection()"

    private static func json<Value: Encodable>(_ value: Value, fallback: String) -> String {
        guard let data = try? JSONEncoder().encode(value), let text = String(data: data, encoding: .utf8) else { return fallback }
        return text
    }
}

extension AnswerComposer {
    /// The notes as the page draws them, in reading order.
    public var pageNotes: [NotesPage.Note] {
        annotations.map { NotesPage.Note(id: $0.id, start: $0.start, end: $0.end, quote: $0.quote) }
    }

    /// What the selection menu offers for `selection`; `nil` when there is nothing to offer: no selection, or a request
    /// whose Mac did not offer notes (`AskView.annotatable`).
    public func annotationOffer(for selection: NotesPage.Selection?) -> NotesPage.Offer? {
        guard ask.annotatable, let selection else { return nil }
        // A passage that is already a note is edited, whatever the form holds: that adds none.
        if let id = selection.overlapId, annotation(id: id) != nil { return .edit(id: id) }
        if annotationLimitReached { return .full }
        return .add(selection)
    }
}
