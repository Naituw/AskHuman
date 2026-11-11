import Foundation
import AHProtocol
import AHCore
import AHStore

// Sample content of the `sessionHistory` scenario (docs/specs/ios-session-history.md): requests of a few Agent
// sessions and what the Home MacBook Pro holds of each of them.

extension MockData {
    /// The keys the Mac would make (`{kind}:{sessionId}`); the sessions are made up.
    public static let longSessionKey = "claude:mock-long-session"
    public static let shortSessionKey = "codex:mock-short-session"
    public static let emptySessionKey = "cursor:mock-empty-session"
    /// Two pages of eight and a few over: the third pull finds the start.
    public static let longSessionCount = 23
    public static let shortSessionCount = 3

    private static let sessionChannels = ["popup", "dingding", "away", "ios", "mute", "slack", "feishu", "telegram"]

    /// A question of a session. `key` is what the Mac's request carries; `nil` is a request that names no session.
    /// The default message is short: the page is shorter than a screen, as most pages are.
    public static func sessionHistoryAsk(
        id: String, key: String?, title: String, message: String = "The tests pass. Shall I open the pull request now?",
        now: Date = Date()
    ) -> RequestView {
        RequestView(
            id: id, kind: "ask", createdAt: ms(now.addingTimeInterval(-30)), agent: AgentRef(kind: "claude", label: "Claude Code"),
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: title, agentKey: key,
            source: "claude", contentHash: "h-" + id,
            ask: AskView(
                isMarkdown: true,
                message: MessageView(text: message, files: []),
                questions: [
                    QuestionView(text: "How should I continue?", options: [
                        OptionView(text: "Open the PR", recommended: true),
                        OptionView(text: "Wait", recommended: false),
                    ]),
                ],
                selectOnly: false, single: true
            )
        )
    }

    /// A permission request of a session (the confirmation page has the thread too).
    public static func sessionHistoryConfirm(id: String, key: String, now: Date = Date()) -> RequestView {
        var view = permissionConfirm(now: now)
        view.id = id
        view.createdAt = ms(now.addingTimeInterval(-30))
        view.sessionTitle = "Plan the cache migration"
        view.agentKey = key
        view.contentHash = "h-" + id
        return view
    }

    /// The scenario's requests: three of the long session (a question, one with a page taller than a screen, and a
    /// permission), one of the short session, one of a session without records, one that names no session, and one of the
    /// Office iMac, whose AskHuman cannot read history.
    public static func sessionHistoryInbox(now: Date = Date()) -> [InboxItem] {
        items([
            (homeMacId, "Home MacBook Pro", sessionHistoryAsk(id: "req-history-long", key: longSessionKey, title: "Plan the cache migration", now: now), true, -30),
            // A page taller than a screen (a long plan to read): the request can stay where it is when the thread comes in.
            (homeMacId, "Home MacBook Pro", sessionHistoryAsk(id: "req-history-tall", key: longSessionKey, title: "Plan the cache migration", message: longSessionMessage, now: now), true, -35),
            (homeMacId, "Home MacBook Pro", sessionHistoryConfirm(id: "req-history-confirm", key: longSessionKey, now: now), true, -40),
            (homeMacId, "Home MacBook Pro", sessionHistoryAsk(id: "req-history-short", key: shortSessionKey, title: "Tidy the README", now: now), true, -50),
            (homeMacId, "Home MacBook Pro", sessionHistoryAsk(id: "req-history-empty", key: emptySessionKey, title: "A brand new session", now: now), true, -60),
            (homeMacId, "Home MacBook Pro", sessionHistoryAsk(id: "req-history-none", key: nil, title: "No session behind this one", now: now), true, -70),
            (officeMacId, "Office iMac", sessionHistoryAsk(id: "req-history-old-mac", key: longSessionKey, title: "On a Mac that cannot read history", now: now), true, -80),
        ], now: now)
    }

    /// What the Home MacBook Pro holds of each session, newest first.
    public static func sessionHistories(now: Date = Date()) -> [String: [SessionExchange]] {
        [
            longSessionKey: (0..<longSessionCount).map { sessionExchange(index: $0, of: longSessionCount, now: now) },
            shortSessionKey: (0..<shortSessionCount).map { sessionExchange(index: $0, of: shortSessionCount, now: now) },
            emptySessionKey: [],
        ]
    }

    /// The `index`-th exchange, newest first, of a session of `count`. Every kind of exchange is among them: Markdown,
    /// several questions with one left unanswered, an automatic reply, Chinese, a message that is far too long for a card,
    /// files and notes, and a question with no message. Each question ends in its number from the oldest (`#1`).
    static func sessionExchange(index: Int, of count: Int, now: Date) -> SessionExchange {
        let number = count - index
        let at = ms(now) - 120_000 - Int64(index) * 3_600_000
        let channel = sessionChannels[index % sessionChannels.count]
        let id = "ex-\(count == longSessionCount ? "long" : "short")-" + (number < 10 ? "0" : "") + String(number)
        func exchange(
            message: String, markdown: Bool = true, attachments: UInt64 = 0, notes: UInt64 = 0, _ qs: [SessionQuestion]
        ) -> SessionExchange {
            SessionExchange(
                id: id, at: at, channel: channel, markdown: markdown, message: message, attachments: attachments, qs: qs,
                notes: notes
            )
        }
        switch index % 8 {
        case 0:
            return exchange(message: "I checked the **cache layer**: the hit rate is 91% and the misses are mostly cold starts.", [
                SessionQuestion(text: "Keep the TTL at ten minutes? #\(number)", selected: ["Yes, keep it"]),
            ])
        case 1:
            return exchange(message: "Three things need your call before I touch the schema.", [
                SessionQuestion(text: "Which database should the migration target? #\(number)", selected: ["Postgres"]),
                SessionQuestion(text: "How should the connections be handled?", input: "Use the existing connection pool"),
                SessionQuestion(text: "Anything I should leave alone?"),
            ])
        case 2:
            return exchange(message: "The tests are green. Shall I merge the branch?", [
                SessionQuestion(
                    text: "Merge now? #\(number)", input: "I am away. Decide on your own, keep going, and do not wait for me."
                ),
            ])
        case 3:
            return exchange(message: "迁移脚本已经写好了，一共 **3 个步骤**，其中第二步会锁表。", [
                SessionQuestion(text: "先在预发环境试跑一遍吗？ #\(number)", selected: ["先试跑"], input: "跑完把耗时发我"),
            ])
        case 4:
            return exchange(message: "Done with the first part. Continue with the rest?", [
                SessionQuestion(text: "Continue? #\(number)", input: "Muted: carry on."),
            ])
        case 5:
            return exchange(message: longSessionMessage, [
                SessionQuestion(text: "Does the plan above look good? #\(number)", selected: ["Looks good"]),
            ])
        case 6:
            return exchange(message: "Here are the screenshots of the failing screens.", attachments: 2, notes: 1, [
                SessionQuestion(text: "Which one should I fix first? #\(number)", input: "The second one", images: 1, files: 1),
            ])
        default:
            return exchange(message: "", [
                SessionQuestion(text: "Anything to add before I stop? #\(number)", input: "No, thanks."),
            ])
        }
    }

    /// A report far too long for a card: it is cut where a card is folded, and shown whole when the card opens.
    private static let longSessionMessage: String = {
        var parts = ["## Migration plan", "The plan has six stages. Each of them can be rolled back on its own."]
        for stage in 1...6 {
            parts.append("### Stage \(stage)")
            parts.append(
                "Stage \(stage) copies a batch of rows into the new table, checks the counts, and only then switches readers over. "
                    + "If the counts differ, the batch is dropped and retried after the backoff, so a bad batch never reaches readers."
            )
            parts.append("- Rows per batch: \(stage * 5_000)\n- Backoff: \(stage * 2) seconds\n- Rollback: drop the batch table")
        }
        parts.append("That is the whole plan.")
        return parts.joined(separator: "\n\n")
    }()
}
