import Foundation
import AHProtocol
import AHCore
import AHStore

extension MockData {
    /// The message of the annotation scenario. Two of its paragraphs are a single word: the UI tests select a word
    /// by pressing on it, and a one-word paragraph is an element of its own that can be pressed on.
    static let annotationsMessage = """
    ## Which cache?

    I compared two candidates for the session cache.

    Redis

    Memcached

    Redis can keep its data on disk when asked to, Memcached cannot. Both are fine for entries that live for ten minutes, \
    so the choice comes down to what else the project will want from the cache later.

    - Redis also offers lists and sorted sets
    - Memcached is simpler to run
    """

    /// A request whose message takes notes, as the newest Macs offer them (`annotatable`). The `markdown` and
    /// `annotatable` switches make the two requests that must not offer them: one the Mac sent as plain text (it is
    /// drawn natively, which takes no notes), and one from a Mac that has no notes.
    public static func annotationsAsk(
        id: String = "req-notes", markdown: Bool = true, annotatable: Bool = true, now: Date = Date()
    ) -> RequestView {
        RequestView(
            id: id, kind: "ask", createdAt: ms(now.addingTimeInterval(-30)), agent: AgentRef(kind: "cursor", label: "Cursor"),
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: "Choose the session cache",
            source: "cursor", contentHash: "h-" + id,
            ask: AskView(
                isMarkdown: markdown,
                message: MessageView(text: annotationsMessage, files: []),
                questions: [
                    QuestionView(text: "Which one should I use?", options: [
                        OptionView(text: "The first", recommended: true),
                        OptionView(text: "The second", recommended: false),
                    ]),
                ],
                selectOnly: false, single: true, annotatable: annotatable
            )
        )
    }

    /// The `annotations` scenario's requests, the newest first: one that takes notes, one drawn as plain text, one
    /// from a Mac that has none.
    public static func annotationsInbox(now: Date = Date()) -> [InboxItem] {
        items([
            (homeMacId, "Home MacBook Pro", annotationsAsk(now: now), true, -30),
            (homeMacId, "Home MacBook Pro", annotationsAsk(id: "req-notes-plain", markdown: false, now: now), true, -60),
            (homeMacId, "Home MacBook Pro", annotationsAsk(id: "req-notes-off", annotatable: false, now: now), true, -90),
        ], now: now)
    }
}
