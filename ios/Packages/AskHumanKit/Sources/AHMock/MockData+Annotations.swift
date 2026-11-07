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

    /// A message taller than the screen, with a one-word paragraph at each end: from the top, a note on the last one is
    /// out of sight, which is where the way back to a passage from the list of notes has something to do.
    static let annotationsTallMessage: String = {
        let filler = String(repeating: "More words about the cache, its keys and how long they live. ", count: 3)
        var parts = ["## A long report", "Alpha"]
        for index in 1...16 {
            parts.append("Section \(index) goes on for a while, so that the end of the report is well down the page. " + filler)
        }
        parts += ["Omega", "That is all."]
        return parts.joined(separator: "\n\n")
    }()

    /// A request whose message takes notes, as the newest Macs offer them (`annotatable`). The `markdown` and
    /// `annotatable` switches make the two requests that must not offer them: one the Mac sent as plain text (it is
    /// drawn natively, which takes no notes), and one from a Mac that has no notes.
    public static func annotationsAsk(
        id: String = "req-notes", markdown: Bool = true, annotatable: Bool = true, message: String? = nil,
        now: Date = Date()
    ) -> RequestView {
        RequestView(
            id: id, kind: "ask", createdAt: ms(now.addingTimeInterval(-30)), agent: AgentRef(kind: "cursor", label: "Cursor"),
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: "Choose the session cache",
            source: "cursor", contentHash: "h-" + id,
            ask: AskView(
                isMarkdown: markdown,
                message: MessageView(text: message ?? annotationsMessage, files: []),
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
    /// from a Mac that has none, and one whose message is taller than the screen.
    public static func annotationsInbox(now: Date = Date()) -> [InboxItem] {
        items([
            (homeMacId, "Home MacBook Pro", annotationsAsk(now: now), true, -30),
            (homeMacId, "Home MacBook Pro", annotationsAsk(id: "req-notes-plain", markdown: false, now: now), true, -60),
            (homeMacId, "Home MacBook Pro", annotationsAsk(id: "req-notes-off", annotatable: false, now: now), true, -90),
            (homeMacId, "Home MacBook Pro", annotationsAsk(id: "req-notes-tall", message: annotationsTallMessage, now: now), true, -120),
        ], now: now)
    }
}
