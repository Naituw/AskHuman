import Foundation
import AHProtocol
import AHCore

/// Sample Agents, watch frames and new-task choices for the demo mode.
extension MockData {
    public static let workingAgentKey = "claude:cc-flaky"
    public static let waitingAgentKey = "cursor:cur-migrate"
    public static let idleAgentKey = "codex:cdx-notes"
    public static let endedAgentKey = "cursor:cur-readme"
    public static let officeAgentKey = "claude:cc-users"
    public static let unsupportedAgentKey = "codex:cdx-leak"

    // The repositories are the ones of the sample todo projects (`MockData+Todos`): the Todos tab shows a dot
    // on a project when an Agent of any Mac works in its repository.
    private static let humanInLoop = ProjectRef(key: "proj-humaninloop", name: "HumanInLoop", repo: "github.com/naituw/askhuman")
    private static let site = ProjectRef(key: "proj-site", name: "askhuman-site")
    private static let billing = ProjectRef(key: "proj-billing", name: "Billing Service", repo: "github.com/acme/billing")

    private static func view(
        _ key: String, title: String, project: ProjectRef, state: String, startedAgo: TimeInterval, activeAgo: TimeInterval,
        elapsed: UInt64 = 0, steps: UInt64 = 0, tool: String? = nil, waiting: String? = nil, preview: String? = nil,
        failed: Bool = false, interject: CanInterject, followed: Bool = false, now: Date
    ) -> AgentView {
        let parts = key.split(separator: ":", maxSplits: 1).map(String.init)
        let label = ["claude": "Claude Code", "cursor": "Cursor", "codex": "Codex"][parts[0]] ?? parts[0]
        return AgentView(
            key: key, kind: parts[0], kindLabel: label, sessionId: parts[1], title: title, project: project, state: state,
            startedAt: ms(now.addingTimeInterval(-startedAgo)), lastActivityAt: ms(now.addingTimeInterval(-activeAgo)),
            activeElapsedSecs: elapsed, turnSteps: steps, currentTool: tool, waitingRequestId: waiting, waitingPreview: preview,
            lastTurnFailed: failed, canInterject: interject, followed: followed
        )
    }

    public static func agents(now: Date = Date()) -> [AgentItem] {
        let home = macs(now: now)[0]
        let office = macs(now: now)[1]
        let running = CanInterject(ok: true)
        let waitingForAnswer = CanInterject(ok: false, reason: "notWorking")
        return [
            AgentItem(macId: home.id, macName: home.name, view: view(
                workingAgentKey, title: "Fix flaky popup tests", project: humanInLoop, state: "working", startedAgo: 1_080, activeAgo: 6,
                elapsed: 1_020, steps: 14, tool: "Edit", interject: running, followed: true, now: now
            )),
            AgentItem(macId: home.id, macName: home.name, view: view(
                waitingAgentKey, title: "Plan the users table migration", project: humanInLoop, state: "waiting", startedAgo: 2_400,
                activeAgo: 95, elapsed: 1_800, steps: 9, waiting: "req-migration", preview: "选择哪种迁移方式？",
                interject: waitingForAnswer, now: now
            )),
            AgentItem(macId: home.id, macName: home.name, view: view(
                idleAgentKey, title: "Write release notes for v0.9", project: site, state: "idle", startedAgo: 5_400, activeAgo: 720,
                elapsed: 900, steps: 5, interject: waitingForAnswer, followed: true, now: now
            )),
            AgentItem(macId: home.id, macName: home.name, view: view(
                endedAgentKey, title: "Update README screenshots", project: humanInLoop, state: "ended", startedAgo: 14_400,
                activeAgo: 7_200, elapsed: 2_100, steps: 31, failed: true, interject: CanInterject(ok: false, reason: "ended"), now: now
            )),
            AgentItem(macId: office.id, macName: office.name, view: view(
                officeAgentKey, title: "Migrate the users table", project: billing, state: "working", startedAgo: 540, activeAgo: 12,
                // The Mac sends "Tool: object" clipped to 80 characters: far wider than a row.
                elapsed: 480, steps: 6, tool: "Shell: cargo test --manifest-path src-tauri/Cargo.toml ios::gateway -- --nocapture…",
                interject: running, followed: true, now: now
            )),
            AgentItem(macId: office.id, macName: office.name, view: view(
                unsupportedAgentKey, title: "Investigate daemon memory growth", project: billing, state: "idle", startedAgo: 7_200,
                activeAgo: 1_900, elapsed: 3_000, steps: 22, interject: CanInterject(ok: false, reason: "unsupported"), now: now
            )),
        ]
    }

    // MARK: watch frames

    /// The frame an Agent shows after `tick` updates: a working one keeps adding steps.
    public static func watchFrame(of agent: AgentView, tick: Int, now: Date = Date()) -> WatchFrameData {
        let at = Int64(now.timeIntervalSince1970)
        switch agent.state {
        case "working":
            let all: [WatchStep] = [
                WatchStep(kind: "read", name: nil, object: "src/views/popup/Inbox.vue", state: "done"),
                WatchStep(kind: "run", name: nil, object: "pnpm test popup", state: "done"),
                WatchStep(kind: "write", name: nil, object: "src/views/popup/Inbox.vue", state: "done"),
                WatchStep(kind: "run", name: nil, object: "pnpm test popup --repeat 3", state: "done"),
                WatchStep(kind: "other", name: "Grep", object: "onMounted", state: "done"),
                WatchStep(kind: "write", name: nil, object: "src/views/popup/InboxSidebar.vue", state: "running"),
            ]
            let shown = min(all.count, 3 + tick)
            var window = Array(all[max(0, shown - 3)..<shown])
            if shown < all.count, let last = window.indices.last { window[last].state = "running" }
            let texts = [
                "我已经定位到不稳定的原因：`Inbox.vue` 在 `onMounted` 里读取了**尚未就绪**的 store。\n\n接下来：\n1. 把读取挪到 `watch` 回调里\n2. 重新跑三遍测试确认不再抖动",
                "修复已经写入，正在连续跑三遍 `pnpm test popup` 验证。如果仍有抖动，我会再检查 `InboxSidebar.vue` 的初始化顺序。",
            ]
            return WatchFrameData(
                phase: "working", title: agent.title, project: agent.project.name, text: texts[min(tick / 2, texts.count - 1)],
                steps: window, stepsOmitted: max(0, shown - 3),
                todos: [
                    WatchTodo(content: "复现不稳定的测试", state: "completed"),
                    WatchTodo(content: "修复 store 的读取时机", state: tick >= 2 ? "completed" : "inProgress"),
                    WatchTodo(content: "连续三遍通过后提交", state: tick >= 2 ? "inProgress" : "pending"),
                ],
                activeElapsedSecs: (agent.activeElapsedSecs) + UInt64(tick) * 2, at: at,
                model: "claude-opus-4-1", reasoning: "high"
            )
        case "waiting":
            return WatchFrameData(
                phase: "waiting", title: agent.title, project: agent.project.name,
                text: "我检查了 `users` 表的现状，有两条路可以走。需要你选择迁移方式后我再继续。",
                steps: [WatchStep(kind: "read", name: nil, object: "migrations/0042_users.sql", state: "done")], stepsOmitted: 6,
                todos: [], activeElapsedSecs: agent.activeElapsedSecs, at: at - 95,
                // Cursor has no reasoning effort: the page shows the model alone.
                model: "composer-1"
            )
        case "ended":
            return WatchFrameData(
                phase: "ended", title: agent.title, project: agent.project.name, text: "截图脚本在第 31 步失败：找不到 `Simulator`。",
                steps: [WatchStep(kind: "run", name: nil, object: "node scripts/screenshots.mjs", state: "failed")],
                todos: [], activeElapsedSecs: agent.activeElapsedSecs, at: at - 7_200
            )
        default:
            return WatchFrameData(
                phase: "idle", title: agent.title, project: agent.project.name,
                text: "发布说明已经写好：\n\n- 新增 iOS 渠道（实验）\n- 修复弹窗在多显示器下的位置问题\n\n需要我继续整理英文版吗？",
                steps: [], todos: [], activeElapsedSecs: agent.activeElapsedSecs, at: at - 720,
                model: "gpt-5.1-codex-max", reasoning: "xhigh"
            )
        }
    }

    /// A latest text as long as the real ones: a paragraph of the Agent's thinking, or its last words (Markdown); or a
    /// thought of one line (`short`), which the preview shows whole.
    public static func watchLatest(_ kind: String) -> WatchLatestText? {
        switch kind {
        case "thinking":
            return WatchLatestText(
                id: "think-1", revision: "think-1", kind: "thinking",
                text: """
                The flaky test fails about one run in five, always in the sidebar snapshot. `Inbox.vue` reads the queue in `onMounted`, before the first sync has landed, so the sidebar sometimes renders an empty list and the snapshot differs.

                Three ways out: move the read into a watch on `store.ready`; await the sync in the test setup; or make the sidebar tolerate an empty queue. The first fixes the cause and keeps the tests honest. The second would hide it, and the third changes the product for the sake of a test.

                `InboxSidebar.vue` reads the queue in its own setup and `PopupFooter.vue` reads the count the same way, so fixing only `Inbox.vue` would just move the flake to the footer test. Better to add a small `useStoreReady()` composable that returns a ref, and read through it in all three places.

                One more thing to check: the fake timers in `inboxCollapse.test.ts` advance 500 ms and might race with the watch callback, so the promises should be flushed after advancing. Write the composable first, then the three call sites, then run the popup suite three times in a row.
                """
            )
        case "answer":
            return WatchLatestText(
                id: "answer-1", revision: "answer-1", kind: "assistant",
                text: """
                修复已经写入，分三步：

                1. 新增 `useStoreReady()`，返回一个在首次同步完成后变为 `true` 的 ref
                2. `Inbox.vue`、`InboxSidebar.vue`、`PopupFooter.vue` 改为都通过它读取队列
                3. 在 `inboxCollapse.test.ts` 里推进假计时器之后补一次 `await flushPromises()`

                连续跑了三遍 `pnpm test popup`，三遍都通过。

                **还没有验证的地方**：

                - Windows 上托盘图标的位置（需要真机）
                - 快速连续到达多条请求时，侧栏的滚动位置

                如果你同意，我下一步去补这两处的测试。
                """
            )
        case "long":
            // About as long as the Mac sends (it keeps at most 4000 bytes of a paragraph): taller than any phone screen.
            return WatchLatestText(
                id: "long-1", revision: "long-1", kind: "assistant",
                text: """
                排查结果已经出来了，先说结论：不稳定的根源是侧栏在首次同步完成之前就读取了队列，所以大约每五次会有一次渲染出空列表。

                **原因**

                `Inbox.vue` 在 `onMounted` 里直接读取队列，这时 `store.ready` 还没有变成 `true`。`InboxSidebar.vue` 与 `PopupFooter.vue` 用的是同样的写法，所以只修其中一处，另外两处的测试仍然会偶尔失败。快照里多出来的那一行空列表占位，正是这个时间差留下的痕迹。

                **修复步骤**

                1. 新增 `useStoreReady()`，返回一个在首次同步完成后变为 `true` 的 ref，并且在组件卸载时自动取消监听
                2. `Inbox.vue`、`InboxSidebar.vue`、`PopupFooter.vue` 改为都通过它读取队列，同步完成之前显示占位，而不是一个空列表
                3. 在 `inboxCollapse.test.ts` 里推进假计时器之后补一次 `await flushPromises()`，避免回调还没有执行断言就已经跑过去了
                4. 把三处各自重复的“等待同步”辅助函数删掉，统一放进 `test/helpers/ready.ts`，以后新增的组件测试直接引用
                5. 给 `store.ready` 补一条单元测试：同步前为 `false`，同步完成后变为 `true`，重复同步不会把它改回去
                6. 弹窗的页脚计数改为在 `ready` 之后才显示，避免先闪一下 0 再跳到真实的数字

                **验证**

                连续跑了三遍 `pnpm test popup`，三遍都通过；又把假计时器的推进量改成 100 毫秒和 2000 毫秒各跑一遍，结果相同。修复前同样的命令在第二遍就会失败一次，说明这几条测试确实能抓到这个问题，而不是碰巧通过。

                **还没有验证的地方**

                - Windows 上托盘图标的位置，需要在真机上看，我这里没有 Windows 环境
                - 快速连续到达多条请求时，侧栏的滚动位置是否会被新到的请求顶开
                - 深色模式下占位骨架的对比度，目前只检查了浅色模式
                - 如果同步一直不完成（比如守护进程没有启动），占位会一直显示，需要决定是否在几秒之后给出提示

                **风险**

                `useStoreReady()` 改变了三个组件的首次渲染时机，凡是依赖“挂载之后立刻就有数据”的快照测试都可能受到影响。我已经更新了弹窗相关的七条快照，其余窗口里的快照没有改动，也没有发现新的差异。另外这次没有碰数据库迁移，也没有改动任何对外的协议字段，回滚时只需要还原这几个组件和测试文件。

                **后续计划**

                如果你同意，我下一步去补上面几处没有验证的测试，并把占位的文案交给你确认。文案我先按“正在同步……”写了一版，英文是 “Syncing…”，你看是否合适；如果要换成别的说法，三种语言的文件我一起改，改完再跑一遍弹窗的整套测试。
                """
            )
        case "short":
            return WatchLatestText(
                id: "short-1", revision: "short-1", kind: "thinking",
                text: "The sidebar reads the queue before the first sync has landed."
            )
        default:
            return nil
        }
    }

    // MARK: new tasks

    /// What a Mac offers for a new task. `models: false` is a Mac that is too old to list any: the
    /// Agents have no `models` or `defaultModel`, and the form has no model or effort row.
    ///
    /// `maxControl: true` is a Mac that controls Cursor's Max mode (an account on the old request-based
    /// plan): the Cursor agent offers the switch and its models say whether they need Max.
    public static func taskOptions(macId: String, models: Bool = true, maxControl: Bool = false) -> TaskOptions {
        if macId == officeMacId {
            return TaskOptions(enabled: false, blocked: "noTerminal", permissionMode: "ask", workspaces: [], agents: [])
        }
        let agents = [
            AgentOption(
                kind: "claude", label: "Claude Code", ready: true, subtitle: "Terminal · Ghostty", permissionSelectable: true,
                models: claudeModels, defaultModel: DefaultModel(label: "opus", efforts: claudeEfforts)
            ),
            // A long list without efforts: the model page has a search field.
            AgentOption(
                kind: "cursor", label: "Cursor", ready: true, subtitle: "Terminal · Ghostty", permissionSelectable: false,
                models: cursorModels, defaultModel: DefaultModel(label: "Auto")
            ),
            // Every model names its own efforts and the one it runs with.
            AgentOption(
                kind: "codex", label: "Codex", ready: true, subtitle: "Codex App", permissionSelectable: true,
                models: codexModels, defaultModel: DefaultModel(label: "GPT-5.1-Codex-Max", efforts: ["low", "medium", "high", "xhigh"], defaultEffort: "high"), speedControl: true
            ),
            // Some models think, some do not: the effort row comes and goes with the model.
            AgentOption(
                kind: "pi", label: "Pi", ready: true, subtitle: "Terminal · Ghostty", permissionSelectable: false,
                models: piModels, defaultModel: DefaultModel(label: "anthropic/claude-sonnet-4-5", efforts: ["low", "medium", "high"])
            ),
            AgentOption(
                kind: "grok", label: "Grok", ready: false, subtitle: "Terminal · Ghostty", permissionSelectable: true,
                reason: "Grok is not installed on this Mac."
            ),
        ].map { agent -> AgentOption in
            guard !models else { return agent }
            var plain = agent
            plain.speedControl = nil
            plain.models = nil
            plain.defaultModel = nil
            return plain
        }
        .map { agent -> AgentOption in
            guard maxControl, agent.kind == "cursor" else { return agent }
            var marked = agent
            marked.maxControl = true
            marked.models = marked.models?.map { model in
                var model = model
                model.maxRequired = cursorMaxMarks[model.id]
                return model
            }
            if marked.defaultModel != nil { marked.defaultModel?.maxRequired = cursorMaxMarks["auto"] }
            return marked
        }
        return TaskOptions(
            enabled: true, permissionMode: "ask",
            workspaces: [
                WorkspaceOption(id: "ws-humaninloop", label: "HumanInLoop", subtitle: "~/Developer/HumanInLoop", pinned: true, lastUsedAt: ms(Date())),
                WorkspaceOption(id: "ws-site", label: "askhuman-site", subtitle: "~/Developer/askhuman-site", pinned: false, lastUsedAt: ms(Date()) - 3_600_000),
                WorkspaceOption(id: "ws-dotfiles", label: "dotfiles", subtitle: "~/.dotfiles", pinned: false, lastUsedAt: ms(Date()) - 86_400_000),
            ],
            agents: agents
        )
    }

    private static let claudeEfforts = ["low", "medium", "high", "xhigh", "max"]

    private static let claudeModels = [
        ModelOption(id: "opus", label: "Opus", note: "\u{2192} claude-opus-4-1", efforts: claudeEfforts),
        ModelOption(id: "sonnet", label: "Sonnet", efforts: claudeEfforts),
        ModelOption(id: "haiku", label: "Haiku", efforts: claudeEfforts),
    ]

    private static let codexModels = [
        ModelOption(
            id: "gpt-5.1-codex-max", label: "GPT-5.1-Codex-Max", note: "The most capable model for long, agentic coding work.",
            efforts: ["low", "medium", "high", "xhigh"], defaultEffort: "medium", fastSupported: true
        ),
        ModelOption(
            id: "gpt-5.1-codex-mini", label: "GPT-5.1-Codex-Mini", note: "Smaller and cheaper, for quicker tasks.",
            efforts: ["medium", "high"], defaultEffort: "medium"
        ),
        ModelOption(id: "gpt-5.1", label: "GPT-5.1", efforts: ["low", "medium", "high"], defaultEffort: "medium"),
    ]

    private static let piModels = [
        ModelOption(id: "anthropic/claude-sonnet-4-5", label: "anthropic/claude-sonnet-4-5", efforts: ["low", "medium", "high"]),
        ModelOption(id: "openai/gpt-5", label: "openai/gpt-5", efforts: ["low", "medium", "high"]),
        ModelOption(id: "ollama/llama3.2", label: "ollama/llama3.2"),
    ]

    /// More than a dozen, as `cursor-agent --list-models` gives.
    private static let cursorModels: [ModelOption] = [
        ("auto", "Auto"), ("composer-1", "Composer 1"), ("gpt-5", "GPT-5"), ("gpt-5-high", "GPT-5 High"),
        ("gpt-5-codex", "GPT-5 Codex"), ("gpt-5-codex-high", "GPT-5 Codex High"), ("gpt-5.1-codex-max", "GPT-5.1 Codex Max"),
        ("gpt-5.1-codex-max-high", "GPT-5.1 Codex Max High"), ("sonnet-4.5", "Sonnet 4.5"),
        ("sonnet-4.5-thinking", "Sonnet 4.5 Thinking"), ("opus-4.1", "Opus 4.1"), ("opus-4.5", "Opus 4.5"),
        ("opus-4.5-thinking", "Opus 4.5 Thinking"), ("haiku-4.5", "Haiku 4.5"), ("gemini-2.5-pro", "Gemini 2.5 Pro"),
        ("gemini-3-pro", "Gemini 3 Pro"), ("grok-4", "Grok 4"), ("grok-code-fast-1", "Grok Code Fast 1"),
        ("kimi-k2", "Kimi K2"), ("qwen3-coder", "Qwen3 Coder"),
    ].map { ModelOption(id: $0.0, label: $0.1) }

    /// Which of the Cursor models need Max, as a Mac on the old plan would say. A model that is not here is
    /// one the Mac cannot tell about (it must not be read as "does not need Max").
    static let cursorMaxMarks: [String: Bool] = [
        "auto": false, "composer-1": false, "gpt-5": false, "gpt-5-high": false, "gpt-5-codex": false,
        "gpt-5.1-codex-max": true, "gpt-5.1-codex-max-high": true, "sonnet-4.5": false, "sonnet-4.5-thinking": false,
        "opus-4.1": true, "grok-4": true,
    ]

    public static func todos(workspaceId: String) -> [TodoOption] {
        guard workspaceId == "ws-humaninloop" else { return [] }
        return [
            TodoOption(id: "todo-1", text: "Add a dark mode toggle to the popup settings", auto: false),
            TodoOption(id: "todo-2", text: "Write a spec for the iOS follow notifications", auto: true, attachments: 1),
            TodoOption(id: "todo-3", text: "Profile the daemon idle CPU on battery", auto: false),
        ]
    }
}
