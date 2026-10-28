import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
import AHProtocol
import AHCore
import AHStore

/// Sample content for previews, screenshots, UI tests and the demo mode of the app.
public enum MockData {
    public static let homeMacId = "mockhomemacmacmacmacmacmac"
    public static let officeMacId = "mockofficemacmacmacmacmac"

    public static func ms(_ date: Date) -> Int64 { Int64(date.timeIntervalSince1970 * 1000) }

    // MARK: Macs

    /// `knowTodos: false` is a pair of Macs whose AskHuman predates the todo messages.
    public static func macs(now: Date = Date(), link: MacLink = .online, knowTodos: Bool = true) -> [MacState] {
        [
            MacState(
                id: homeMacId, name: "Home MacBook Pro", macName: "Home MacBook Pro", alias: nil, link: link,
                caps: Caps(newTask: true, interject: true, watch: true, todos: knowTodos),
                slot: SlotInfo(active: false, autoActivation: true),
                pairedAt: now.addingTimeInterval(-86_400 * 12), lastSeenAt: now, pendingCount: 0, pushStatus: "ok"
            ),
            MacState(
                id: officeMacId, name: "Office iMac", macName: "wutian's iMac", alias: "Office iMac", link: link,
                caps: Caps(newTask: true, interject: true, watch: true, todos: knowTodos),
                slot: SlotInfo(active: true, autoActivation: false),
                pairedAt: now.addingTimeInterval(-86_400 * 3), lastSeenAt: now.addingTimeInterval(link == .online ? 0 : -3_600),
                pendingCount: 0, pushStatus: "ok"
            ),
        ]
    }

    // MARK: requests

    private static let cursor = AgentRef(kind: "cursor", label: "Cursor")
    private static let claude = AgentRef(kind: "claude", label: "Claude Code")
    private static let codex = AgentRef(kind: "codex", label: "Codex")

    private static func hash(_ id: String) -> String { "h-" + id }

    /// A multi-question request with a Markdown message, attached files and recommendations.
    public static func migrationAsk(now: Date = Date()) -> RequestView {
        let message = """
        ## 数据库迁移方案

        我检查了 `users` 表的现状，有两条路可以走：

        | 方案 | 停机 | 风险 |
        |:--|--:|:--|
        | 在线双写 | 0 分钟 | 回滚复杂 |
        | 停机迁移 | 约 8 分钟 | 低 |

        > 生产库约 **2.1 亿**行，上一次类似迁移用了 7 分 40 秒。

        ```sql
        ALTER TABLE users ADD COLUMN locale TEXT NOT NULL DEFAULT 'en';
        ```

        完整的评估见附件。
        """
        return RequestView(
            id: "req-migration", kind: "ask", createdAt: ms(now.addingTimeInterval(-95)), agent: cursor,
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: "Plan the users table migration",
            source: "cursor", contentHash: hash("migration"),
            ask: AskView(
                isMarkdown: true,
                message: MessageView(text: message, files: [
                    AssetRef(assetId: "asset-plan", name: "migration-plan.md", size: 4_812, mime: "text/markdown", isImage: false),
                    AssetRef(assetId: "asset-chart", name: "latency.png", size: 183_422, mime: "image/png", isImage: true, thumb: chartThumb),
                    AssetRef(assetId: "asset-diff", name: "schema.diff", size: 2_204, mime: "text/x-diff", isImage: false),
                ]),
                questions: [
                    QuestionView(text: "选择哪种迁移方式？", options: [
                        OptionView(text: "在线双写", recommended: true),
                        OptionView(text: "停机迁移", recommended: false),
                        OptionView(text: "先在影子库演练", recommended: false),
                    ]),
                    QuestionView(text: "需要同时回填历史数据吗？可多选。", options: [
                        OptionView(text: "回填 `locale`", recommended: true),
                        OptionView(text: "回填 `timezone`", recommended: false),
                        OptionView(text: "不回填", recommended: false),
                    ]),
                    QuestionView(text: "还有什么需要我注意的？", options: []),
                ],
                selectOnly: false, single: false
            )
        )
    }

    // MARK: Markdown showcase

    /// The Markdown showcase's message: soft line breaks, inline styles, a picture, a table with long
    /// cells, a Mermaid diagram, code, a quote and nested lists.
    private static var showcaseMessage: String {
        #"""
        ## 迁移评估报告

        **背景**
        我检查了 `orders` 表的现状，并比较了两种迁移方案。
        这一行本来是同一段里的下一行，Mac 弹窗会换行显示。
        第三行也一样。

        行内样式：**粗体**、*斜体*、~~删除线~~、`inline_code()`，链接 https://example.com/docs ，以及 [带文字的链接](https://example.com/guide)。

        ![架构示意（内联图片）](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAPAAAABICAIAAABHru/dAAABM0lEQVR42u3SMQ3AIBRAQZbaqArGTrVQTWwIQQSy2FhqooHk95Kn4OXSeVUpTMkCBQR9l7G43J4tzX5saf3hv30GGmigjQYaaKCBBhpon4EGGmiggQbaaKCBBhpooIH2GWiggTYaaKCNBhpooH0GGmifgQYaaKOBBtpooIEG2meggfYZaKCBNhpooI0GGmigfQYaaJ+BBhpoo4EG2miggQbaZ6CB9hlooIE2GmigjQYaaKB9Bhpon4EGGmijgQbaaKCBBtpnoIEGGmiggTYaaKCBBhpooH0GGmiggQYaaKOBBhpon4EG2meggQbaaKCBNhpooIH2GWigfQYaaKCNBhpoo4EGGmifgQbaZ6CBBtpooIE2GmiggfYZaKB9BhpooI0GGmijgQYaaJ+/Ay3FCGiF6gUrYUkUUaJdygAAAABJRU5ErkJggg==)

        ### 方案对比

        | 方案 | 停机 | 风险 | 说明 |
        |:--|--:|:--|:--|
        | 在线双写 | 0 分钟 | 回滚复杂 | 先双写新旧两张表，校验一致后再切读流量；期间需要额外的监控与对账任务，回滚时要处理已写入新表的增量数据 |
        | 停机迁移 | 约 8 分钟 | 低 | 维护窗口内一次完成，过程简单可控，但需要提前公告并暂停写入 |

        ### 流程

        ```mermaid
        flowchart TD
          A[收到变更请求] --> B{是否需要停机?}
          B -- 是 --> C[公告维护窗口]
          B -- 否 --> D[开启双写]
          C --> E[执行迁移]
          D --> E
          E --> F[校验数据一致]
          F --> G((完成))
        ```

        ### 命令

        ```bash
        ./scripts/migrate.sh --table orders --batch 5000 --dry-run
        ```

        > 生产库约 **2.1 亿**行，上一次类似迁移用了 7 分 40 秒。

        1. 先在影子库演练
        2. 观察复制延迟（超过 30 秒就暂停）
           - 指标：`replica_lag_seconds`
           - 告警：值班群
        3. 切换读流量
        """#
    }

    /// A request that shows the Markdown rendering: soft line breaks, inline styles, a picture, a table
    /// with long cells, a Mermaid diagram, code, a quote and nested lists.
    public static func markdownAsk(now: Date = Date()) -> RequestView {
        let message = showcaseMessage
        return RequestView(
            id: "req-markdown", kind: "ask", createdAt: ms(now.addingTimeInterval(-30)), agent: cursor,
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: "Plan the orders table migration",
            source: "cursor", contentHash: hash("markdown"),
            ask: AskView(
                isMarkdown: true,
                message: MessageView(text: message, files: []),
                questions: [
                    QuestionView(text: "选择哪种迁移方式？可以参考上面的**对比表**和流程图。", options: [
                        OptionView(text: "在线双写", recommended: true),
                        OptionView(text: "停机迁移", recommended: false),
                    ]),
                    QuestionView(text: "回填历史数据时批大小怎么定？\n建议先用 `5000`，观察复制延迟后再调整；超过 **30 秒**就暂停。", options: [
                        OptionView(text: "5000", recommended: true),
                        OptionView(text: "20000", recommended: false),
                    ]),
                    QuestionView(text: "还有什么需要我注意的？", options: []),
                ],
                selectOnly: false, single: false
            )
        )
    }

    public static func markdownOnly(now: Date = Date()) -> [InboxItem] {
        items([(homeMacId, "Home MacBook Pro", markdownAsk(now: now), true, -30)], now: now)
    }

    /// The showcase's tall message over two short select-only questions. The questions fit on the screen
    /// while the message is still a one-line placeholder (a message drawn by a web page has no height
    /// until the page says so): they must not count as read just because they looked close.
    public static func tallMessageAsk(now: Date = Date()) -> RequestView {
        RequestView(
            id: "req-tall", kind: "ask", createdAt: ms(now.addingTimeInterval(-30)), agent: cursor,
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: "Plan the orders table migration",
            source: "cursor", contentHash: hash("tall"),
            ask: AskView(
                isMarkdown: true,
                message: MessageView(text: showcaseMessage, files: []),
                questions: [
                    QuestionView(text: "Which way?", options: [
                        OptionView(text: "Online", recommended: true),
                        OptionView(text: "Offline", recommended: false),
                    ]),
                    QuestionView(text: "Batch size?", options: [
                        OptionView(text: "5000", recommended: true),
                        OptionView(text: "20000", recommended: false),
                    ]),
                ],
                selectOnly: true, single: true
            )
        )
    }

    public static func tallMessageOnly(now: Date = Date()) -> [InboxItem] {
        items([(homeMacId, "Home MacBook Pro", tallMessageAsk(now: now), true, -30)], now: now)
    }

    /// A select-only form of three questions that does not fit on one screen: "Send" waits for a
    /// choice in each of them, and the hint under the count leads to the first one still open.
    public static func strictAsk(now: Date = Date()) -> RequestView {
        RequestView(
            id: "req-strict", kind: "ask", createdAt: ms(now.addingTimeInterval(-60)), agent: claude,
            project: ProjectRef(key: "proj-relay", name: "relay"), sessionTitle: "Choose the release settings",
            source: "claude", contentHash: hash("strict"),
            ask: AskView(
                isMarkdown: true,
                message: MessageView(text: "发布前需要确认三项设置。", files: []),
                questions: [
                    QuestionView(text: "发布到哪个渠道？", options: [
                        OptionView(text: "Stable", recommended: true),
                        OptionView(text: "Beta", recommended: false),
                        OptionView(text: "Nightly", recommended: false),
                        OptionView(text: "仅内部测试", recommended: false),
                        OptionView(text: "暂不发布", recommended: false),
                    ]),
                    QuestionView(text: "是否同时发布更新日志？", options: [
                        OptionView(text: "发布，并通知订阅者", recommended: true),
                        OptionView(text: "只发布，不通知", recommended: false),
                        OptionView(text: "不发布", recommended: false),
                        OptionView(text: "稍后再说", recommended: false),
                        OptionView(text: "先给我看草稿", recommended: false),
                    ]),
                    QuestionView(text: "发布后要在哪里通知团队？", options: [
                        OptionView(text: "Slack #releases", recommended: true),
                        OptionView(text: "邮件列表", recommended: false),
                        OptionView(text: "两者都要", recommended: false),
                        OptionView(text: "不用通知", recommended: false),
                        OptionView(text: "我自己来", recommended: false),
                    ]),
                ],
                selectOnly: true, single: true
            )
        )
    }

    public static func strictOnly(now: Date = Date()) -> [InboxItem] {
        items([(homeMacId, "Home MacBook Pro", strictAsk(now: now), true, -60)], now: now)
    }

    public static func selectOnlyAsk(now: Date = Date()) -> RequestView {
        RequestView(
            id: "req-select", kind: "ask", createdAt: ms(now.addingTimeInterval(-600)), agent: claude,
            project: ProjectRef(key: "proj-relay", name: "relay"), source: "claude", contentHash: hash("select"),
            ask: AskView(
                isMarkdown: true,
                message: MessageView(text: "测试在 `node 20` 上通过，在 `node 22` 上有 2 个失败。", files: []),
                questions: [
                    QuestionView(text: "要把 CI 矩阵固定在 Node 20 吗？", options: [
                        OptionView(text: "固定在 Node 20", recommended: true),
                        OptionView(text: "继续修复 Node 22", recommended: false),
                    ]),
                ],
                selectOnly: true, single: true
            )
        )
    }

    public static func whatsNextAsk(now: Date = Date()) -> RequestView {
        RequestView(
            id: "req-next", kind: "ask", createdAt: ms(now.addingTimeInterval(-3_000)), agent: codex,
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: "收尾 iOS 通知与分组的细节",
            source: "codex", contentHash: hash("next"),
            ask: AskView(
                isMarkdown: true,
                message: MessageView(text: "已完成：配对流程的单元测试与端到端测试。", files: []),
                questions: [
                    QuestionView(text: "接下来做什么？", options: [
                        OptionView(text: "⚡ 补全 Agents 列表的空态", recommended: false, todo: true),
                        OptionView(text: "实现通知扩展的解密", recommended: true, todo: true),
                        OptionView(text: "整理 Markdown 渲染的快照测试", recommended: false, todo: true),
                    ]),
                ],
                selectOnly: false, single: true, whatsNext: true
            )
        )
    }

    public static func permissionConfirm(now: Date = Date()) -> RequestView {
        RequestView(
            id: "req-permission", kind: "confirm", createdAt: ms(now.addingTimeInterval(-40)), agent: claude,
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: "Fix the flaky login test",
            source: "claude", contentHash: hash("permission"),
            confirm: ConfirmView(
                title: "允许运行这条命令吗？",
                context: [
                    ConfirmFieldView(id: "tool", label: "工具", value: "Bash", kind: "text"),
                    ConfirmFieldView(id: "cwd", label: "目录", value: "~/Developer/HumanInLoop", kind: "path"),
                    ConfirmFieldView(id: "at", label: "时间", value: "\(ms(now.addingTimeInterval(-40)))", kind: "timestamp"),
                ],
                detail: ConfirmDetailView(
                    summary: "git push --force-with-lease origin feature/ios",
                    bodyMd: "```bash\ngit push --force-with-lease origin feature/ios\n```\n\n将覆盖远端分支上的 3 个提交。"
                ),
                choices: [
                    ConfirmChoiceView(wireIndex: 0, label: "允许一次", description: "只允许这一次", role: "primary", recommended: true),
                    ConfirmChoiceView(wireIndex: 2, label: "拒绝", description: "告诉 Agent 不要执行", role: "danger", recommended: false),
                ],
                defaultIndex: nil,
                input: ConfirmInputView(
                    id: "reason", label: "拒绝理由（可选）", placeholder: "例如：先在本地跑一遍测试", maxChars: 500,
                    visibleWhenIndex: 2, required: false
                ),
                dismissIndex: 2, submitLabel: "提交",
                expiresAt: ms(now.addingTimeInterval(540))
            )
        )
    }

    public static func stopConfirm(now: Date = Date()) -> RequestView {
        RequestView(
            id: "req-stop", kind: "confirm", createdAt: ms(now.addingTimeInterval(-7_200)), agent: cursor,
            project: ProjectRef(key: "proj-relay", name: "relay"), source: "cursor", contentHash: hash("stop"),
            confirm: ConfirmView(
                title: "Agent 想结束本轮工作",
                context: [ConfirmFieldView(id: "turns", label: "轮次", value: "14", kind: "text")],
                detail: ConfirmDetailView(summary: "已完成限流与错误映射，测试全部通过。", bodyMd: "已完成限流与错误映射，测试全部通过。"),
                choices: [
                    ConfirmChoiceView(wireIndex: 0, label: "结束", description: "", role: "primary", recommended: true),
                    ConfirmChoiceView(wireIndex: 1, label: "继续工作", description: "让 Agent 再检查一遍", role: "default", recommended: false),
                ],
                defaultIndex: nil,
                input: ConfirmInputView(
                    id: "more", label: "补充说明", placeholder: "还需要做什么？", maxChars: 800, visibleWhenIndex: 1, required: true
                ),
                dismissIndex: 0, submitLabel: "提交",
                expiresAt: ms(now.addingTimeInterval(86_400))
            )
        )
    }

    public static func inbox(now: Date = Date()) -> [InboxItem] {
        items([
            (homeMacId, "Home MacBook Pro", permissionConfirm(now: now), true, -40),
            (homeMacId, "Home MacBook Pro", migrationAsk(now: now), true, -95),
            (officeMacId, "Office iMac", selectOnlyAsk(now: now), false, -600),
            (homeMacId, "Home MacBook Pro", whatsNextAsk(now: now), false, -3_000),
            (officeMacId, "Office iMac", stopConfirm(now: now), false, -7_200),
        ], now: now)
    }

    /// Two long questions, both unread: the migration plan and a copy of it for another table.
    public static func longPair(now: Date = Date()) -> [InboxItem] {
        var second = migrationAsk(now: now)
        second.id = "req-migration-2"
        second.sessionTitle = "Plan the orders table migration"
        second.contentHash = hash("migration-2")
        return items([
            (homeMacId, "Home MacBook Pro", migrationAsk(now: now), true, -95),
            (homeMacId, "Home MacBook Pro", second, true, -120),
        ], now: now)
    }

    private static func items(_ entries: [(String, String, RequestView, Bool, TimeInterval)], now: Date) -> [InboxItem] {
        entries.map { macId, macName, view, unread, offset in
            InboxItem(
                macId: macId, macName: macName, requestId: view.id,
                summary: RequestSummary(
                    id: view.id, kind: view.kind, project: view.project, agent: view.agent, sessionTitle: view.sessionTitle,
                    createdAt: view.createdAt, preview: preview(of: view), contentHash: view.contentHash, size: 2_048
                ),
                view: view, receivedAt: now.addingTimeInterval(offset), unread: unread, state: .pending
            )
        }
    }

    static func preview(of view: RequestView) -> String {
        let source = view.ask?.message.text ?? view.confirm?.detail.summary ?? ""
        let line = source.split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty } ?? ""
        return String(line.prefix(120))
    }

    // MARK: history

    public static func history(now: Date = Date()) -> [HistoryEntry] {
        let earlier = RequestView(
            id: "req-old-1", kind: "ask", createdAt: ms(now.addingTimeInterval(-90_000)), agent: claude,
            project: ProjectRef(key: "proj-relay", name: "relay"), sessionTitle: "Cache the APNs token",
            source: "claude", contentHash: hash("old1"),
            ask: AskView(
                isMarkdown: true,
                message: MessageView(text: "要不要把 APNs 令牌缓存 50 分钟？", files: []),
                questions: [QuestionView(text: "缓存策略", options: [
                    OptionView(text: "缓存 50 分钟", recommended: true), OptionView(text: "每次都签名", recommended: false),
                ])],
                selectOnly: false, single: true
            )
        )
        let elsewhere = RequestView(
            id: "req-old-2", kind: "confirm", createdAt: ms(now.addingTimeInterval(-170_000)), agent: cursor,
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), source: "cursor", contentHash: hash("old2"),
            confirm: permissionConfirm(now: now).confirm
        )
        return [
            HistoryEntry(
                id: CachedRequest.key(homeMacId, earlier.id), macId: homeMacId, macName: "Home MacBook Pro", request: earlier,
                outcome: .answered, answers: [AnswerItem(q: 0, sel: [0])],
                receivedAt: now.addingTimeInterval(-90_000), finishedAt: now.addingTimeInterval(-89_940)
            ),
            HistoryEntry(
                id: CachedRequest.key(officeMacId, elsewhere.id), macId: officeMacId, macName: "Office iMac", request: elsewhere,
                outcome: .answeredElsewhere, by: "Mac 弹窗",
                receivedAt: now.addingTimeInterval(-170_000), finishedAt: now.addingTimeInterval(-169_900)
            ),
        ]
    }

    // MARK: files

    /// Bytes for a mock attachment: a chart for images, text for the rest.
    public static func contents(of asset: AssetRef) -> Data {
        if asset.isImage { return chartPNG() ?? Data() }
        switch asset.name {
        case "schema.diff":
            return Data("""
            --- a/schema.sql
            +++ b/schema.sql
            @@ -12,6 +12,7 @@ CREATE TABLE users (
               id BIGINT PRIMARY KEY,
               email TEXT NOT NULL,
            -  name TEXT
            +  name TEXT NOT NULL DEFAULT '',
            +  locale TEXT NOT NULL DEFAULT 'en',
               created_at TIMESTAMPTZ NOT NULL
             );
            """.utf8)
        default:
            return Data("""
            # 迁移评估

            ## 结论

            推荐 **在线双写**：零停机，代价是回滚更复杂。

            - 双写窗口：2 天
            - 校验：每小时抽样 10 万行
            - 回滚：关闭双写开关即可

            | 阶段 | 耗时 |
            |:--|--:|
            | 准备 | 1 天 |
            | 双写 | 2 天 |
            | 切换 | 10 分钟 |
            """.utf8)
        }
    }

    static func chartPNG() -> Data? { chart(width: 640, height: 360, type: .png) }

    /// A small JPEG preview of the chart, as a Mac attaches to an image asset.
    static let chartThumb: String = chart(width: 192, height: 108, type: .jpeg).map(Bytes.b64u) ?? ""

    private static func chart(width: Int, height: Int, type: UTType) -> Data? {
        guard let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return nil }
        let scale = CGFloat(width) / 640
        context.setFillColor(CGColor(red: 0.96, green: 0.97, blue: 1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        let bars: [CGFloat] = [0.35, 0.5, 0.42, 0.7, 0.62, 0.88, 0.75]
        for (index, value) in bars.enumerated() {
            context.setFillColor(CGColor(red: 0.2, green: 0.45 + CGFloat(index) * 0.05, blue: 0.95, alpha: 1))
            context.fill(CGRect(x: (40 + CGFloat(index) * 85) * scale, y: 30 * scale, width: 60 * scale, height: value * 280 * scale))
        }
        guard let image = context.makeImage() else { return nil }
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(data, type.identifier as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return data as Data
    }
}
