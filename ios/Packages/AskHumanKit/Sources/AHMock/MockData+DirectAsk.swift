import Foundation
import AHProtocol
import AHCore
import AHStore

extension MockData {
    /// A request of the `direct` scenario. The message is empty unless one is given, so the question is all there is:
    /// the phone shows a request like that with the question in the message area (`AskView.isDirectAsk`).
    private static func directAsk(
        _ id: String, title: String, questions: [QuestionView], message: String = "", markdown: Bool = true,
        annotatable: Bool = true, selectOnly: Bool = false, whatsNext: Bool = false, now: Date
    ) -> RequestView {
        RequestView(
            id: id, kind: "ask", createdAt: ms(now.addingTimeInterval(-30)), agent: AgentRef(kind: "cursor", label: "Cursor"),
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: title,
            source: "cursor", contentHash: "h-" + id,
            ask: AskView(
                isMarkdown: markdown, message: MessageView(text: message, files: []), questions: questions,
                selectOnly: selectOnly, single: questions.count == 1, whatsNext: whatsNext, annotatable: annotatable
            )
        )
    }

    private static func choice(_ first: String, _ second: String) -> [OptionView] {
        [OptionView(text: first, recommended: true), OptionView(text: second, recommended: false)]
    }

    /// The `direct` scenario's requests, the newest first. The first five are one question with no message (a short one,
    /// a long one that takes notes, a strict choice, plain text, and one from a Mac that has no notes); the last three are
    /// not (What's next, two questions, and a message with a question) and keep their questions in their cards.
    public static func directInbox(now: Date = Date()) -> [InboxItem] {
        let entries: [(String, String, RequestView, Bool, TimeInterval)] = [
            (homeMacId, "Home MacBook Pro", directAsk(
                "req-direct-short", title: "Deploy to staging",
                questions: [QuestionView(text: "Should I carry on deploying to staging?", options: choice("Carry on", "Not yet"))],
                now: now
            ), true, -10),
            (homeMacId, "Home MacBook Pro", directAsk(
                "req-direct-long", title: "Choose the session cache",
                questions: [QuestionView(text: annotationsMessage, options: choice("The first", "The second"))],
                now: now
            ), true, -20),
            (homeMacId, "Home MacBook Pro", directAsk(
                "req-direct-strict", title: "Pick the environment",
                questions: [QuestionView(text: "Which environment should the release go to?", options: choice("Staging", "Production"))],
                annotatable: false, selectOnly: true, now: now
            ), true, -30),
            (homeMacId, "Home MacBook Pro", directAsk(
                "req-direct-plain", title: "Plain question",
                questions: [QuestionView(text: "Plain text, no Markdown: may I carry on?", options: choice("Carry on", "Not yet"))],
                markdown: false, now: now
            ), true, -40),
            (homeMacId, "Home MacBook Pro", directAsk(
                "req-direct-old", title: "From an older Mac",
                questions: [QuestionView(text: "Can I carry on from here?", options: choice("Carry on", "Not yet"))],
                annotatable: false, now: now
            ), true, -50),
            (homeMacId, "Home MacBook Pro", directAsk(
                "req-direct-next", title: "What next",
                questions: [QuestionView(text: "What should I do next?", options: choice("Write the tests", "Update the docs"))],
                whatsNext: true, now: now
            ), true, -60),
            (homeMacId, "Home MacBook Pro", directAsk(
                "req-direct-two", title: "Two questions",
                questions: [
                    QuestionView(text: "Which branch should I use?", options: choice("main", "develop")),
                    QuestionView(text: "Which target should I build?", options: choice("debug", "release")),
                ],
                now: now
            ), true, -70),
            (homeMacId, "Home MacBook Pro", directAsk(
                "req-direct-message", title: "Message and question",
                questions: [QuestionView(text: "Deploy to production now?", options: choice("Deploy", "Wait"))],
                message: "Staging is healthy: all 42 checks passed.", now: now
            ), true, -80),
        ]
        return items(entries, now: now)
    }
}
