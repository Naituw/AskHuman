import Foundation
import AHStore

extension AnswerComposer {
    /// A note as the list of notes on the answer screen shows it (docs/specs/message-annotations.md §7.4).
    public struct AnnotationRow: Identifiable, Equatable, Sendable {
        public let id: Int
        /// The number the passage has on the message page: the page numbers the passages it found, from 1, in reading
        /// order. `nil` for a note whose passage the page could not find (the message was drawn differently, or not
        /// at all): it has no number there, and the list does not make one up (N3).
        public let number: Int?
        /// The passage as it was when the note was written.
        public let quote: String
        public let note: String
    }

    /// The notes for the list, in reading order, those without a passage last. The numbers are the page's: a note that has
    /// none is skipped by them, so the notes after it count on from the one before.
    public var annotationRows: [AnnotationRow] {
        var numbered = 0
        return annotations.map { item in
            guard isLocated(item.id) else { return AnnotationRow(id: item.id, number: nil, quote: item.quote, note: item.note) }
            numbered += 1
            return AnnotationRow(id: item.id, number: numbered, quote: item.quote, note: item.note)
        }
    }
}
