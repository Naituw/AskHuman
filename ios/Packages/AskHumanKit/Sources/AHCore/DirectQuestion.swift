import Foundation
import AHProtocol

extension AskView {
    /// A request that is one question and no words besides: no text in the message, exactly one question, and not
    /// What's next. The phone shows such a question in the message area, above the options, where a message would be: so
    /// it takes notes there like one (docs/specs/message-annotations.md N1, D17), whatever its length.
    ///
    /// Files attached to the message do not count as a message: they are not the Agent's words, and a note needs words to
    /// refer to. The files stay in the message area, below the question (D2, 2026-11-16).
    ///
    /// Only these three things decide it. Not whether the Mac offers notes, whether the text is Markdown, or whether the
    /// choice is strict: the same request looks the same on every Mac, and a question that cannot take a note is
    /// still where a person looks for it.
    public var isDirectAsk: Bool {
        !whatsNext && questions.count == 1
            && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// What the message area has to say, trimmed: the message, or, for a direct ask, its question. Empty when there is
    /// no text to put there (several questions and no message, or a question whose text is empty). The files of the message
    /// are not part of it; the message area lists them under this text.
    public var messageAreaText: String {
        let text = isDirectAsk ? questions[0].text : message.text
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
