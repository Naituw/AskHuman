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

    private static let humanInLoop = ProjectRef(key: "proj-humaninloop", name: "HumanInLoop")
    private static let site = ProjectRef(key: "proj-site", name: "askhuman-site")
    private static let billing = ProjectRef(key: "proj-billing", name: "Billing Service")

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
                activeElapsedSecs: (agent.activeElapsedSecs) + UInt64(tick) * 2, at: at
            )
        case "waiting":
            return WatchFrameData(
                phase: "waiting", title: agent.title, project: agent.project.name,
                text: "我检查了 `users` 表的现状，有两条路可以走。需要你选择迁移方式后我再继续。",
                steps: [WatchStep(kind: "read", name: nil, object: "migrations/0042_users.sql", state: "done")], stepsOmitted: 6,
                todos: [], activeElapsedSecs: agent.activeElapsedSecs, at: at - 95
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
                steps: [], todos: [], activeElapsedSecs: agent.activeElapsedSecs, at: at - 720
            )
        }
    }

    // MARK: new tasks

    public static func taskOptions(macId: String) -> TaskOptions {
        if macId == officeMacId {
            return TaskOptions(enabled: false, blocked: "noTerminal", permissionMode: "ask", workspaces: [], agents: [])
        }
        return TaskOptions(
            enabled: true, permissionMode: "ask",
            workspaces: [
                WorkspaceOption(id: "ws-humaninloop", label: "HumanInLoop", subtitle: "~/Developer/HumanInLoop", pinned: true, lastUsedAt: ms(Date())),
                WorkspaceOption(id: "ws-site", label: "askhuman-site", subtitle: "~/Developer/askhuman-site", pinned: false, lastUsedAt: ms(Date()) - 3_600_000),
                WorkspaceOption(id: "ws-dotfiles", label: "dotfiles", subtitle: "~/.dotfiles", pinned: false, lastUsedAt: ms(Date()) - 86_400_000),
            ],
            agents: [
                AgentOption(kind: "claude", label: "Claude Code", ready: true, subtitle: "Terminal · Ghostty", permissionSelectable: true),
                AgentOption(kind: "cursor", label: "Cursor", ready: true, subtitle: "Terminal · Ghostty", permissionSelectable: false),
                AgentOption(
                    kind: "codex", label: "Codex", ready: false, subtitle: "Codex App", permissionSelectable: true,
                    reason: "Open the Codex App on the Mac first."
                ),
            ]
        )
    }

    public static func todos(workspaceId: String) -> [TodoOption] {
        guard workspaceId == "ws-humaninloop" else { return [] }
        return [
            TodoOption(id: "todo-1", text: "Add a dark mode toggle to the popup settings", auto: false),
            TodoOption(id: "todo-2", text: "Write a spec for the iOS follow notifications", auto: true, attachments: 1),
            TodoOption(id: "todo-3", text: "Profile the daemon idle CPU on battery", auto: false),
        ]
    }
}
