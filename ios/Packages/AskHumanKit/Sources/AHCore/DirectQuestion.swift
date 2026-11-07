import Foundation
import AHProtocol

extension AskView {
    /// A request that is one question and nothing else: no text in the message, no files, exactly one question, and not
    /// What's next. The phone shows such a question in the message area, above the options, where a message would be: so
    /// it takes notes there like one (docs/specs/message-annotations.md N1, D17), whatever its length.
    ///
    /// Only these four things decide it. Not whether the Mac offers notes, whether the text is Markdown, or whether the
    /// choice is strict: the same request looks the same on every Mac, and a question that cannot take a note is
    /// still where a person looks for it.
    public var isDirectAsk: Bool {
        !whatsNext && message.files.isEmpty && questions.count == 1
            && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// What the message area has to say, trimmed: the message, or, for a direct ask, its question. Empty when there is
    /// nothing to put there (only files, or a question whose text is empty).
    public var messageAreaText: String {
        let text = isDirectAsk ? questions[0].text : message.text
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
