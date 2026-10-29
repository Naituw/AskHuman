import Foundation
import AHProtocol
import AHCore

/// Sample todos for the demo mode. The Home MacBook Pro syncs its todos through iCloud, the Office iMac
/// keeps its own: that shows both kinds of source.
extension MockData {
    public static let todoAskHumanProject = "a1b2c3d4e5f60001"
    public static let todoBillingProject = "a1b2c3d4e5f60002"
    public static let todoBlogProject = "a1b2c3d4e5f60003"
    public static let todoNotesProject = "a1b2c3d4e5f60004"
    public static let todoDotfilesProject = "a1b2c3d4e5f60005"
    public static let todoScratchProject = "a1b2c3d4e5f60006"
    public static let todoOfficeWorkProject = "b1b2c3d4e5f60001"
    public static let todoOfficePlayProject = "b1b2c3d4e5f60002"
    public static let todoOfficeSandboxProject = "b1b2c3d4e5f60003"
    /// A project only the Office iMac has once it syncs too (no repository, so nothing of it syncs).
    public static let todoOfficeIntranetProject = "b1b2c3d4e5f60004"

    /// What the sample Macs say about their todos: the projects (rows) and the todos of each.
    public struct TodoSample {
        public var rows: [String: [TodoProjectRow]]
        /// The rows a Mac sends instead of `rows` once it syncs through iCloud (`mode: icloud`).
        public var syncedRows: [String: [TodoProjectRow]]
        public var lists: [String: ProjectTodos]
    }

    public static func todoSample(now: Date = Date()) -> TodoSample {
        func at(_ ago: TimeInterval) -> UInt64 { UInt64(max(0, now.addingTimeInterval(-ago).timeIntervalSince1970 * 1000)) }
        var rows: [String: [TodoProjectRow]] = [:]
        var syncedRows: [String: [TodoProjectRow]] = [:]
        var lists: [String: ProjectTodos] = [:]

        @discardableResult
        func list(_ mac: String, _ row: TodoProjectRow, pending: [TodoItem] = [], history: [TodoDoneItem] = [], canAuto: Bool = true) -> TodoProjectRow {
            var row = row
            row.pending = UInt64(pending.count)
            row.history = UInt64(history.count)
            lists[mac + "|" + row.id] = ProjectTodos(
                id: row.id, rev: 1, name: row.name, canAuto: canAuto, pending: pending, history: history
            )
            return row
        }

        func add(_ mac: String, _ row: TodoProjectRow, pending: [TodoItem] = [], history: [TodoDoneItem] = [], canAuto: Bool = true) {
            rows[mac, default: []].append(list(mac, row, pending: pending, history: history, canAuto: canAuto))
        }

        // Home MacBook Pro: iCloud sync is on.
        add(
            homeMacId,
            TodoProjectRow(
                id: todoAskHumanProject, name: "AskHuman", repo: "github.com/naituw/askhuman", scope: "synced", section: "synced",
                agent: "working", macs: ["Home MacBook Pro", "Office iMac"]
            ),
            pending: [
                TodoItem(id: "todo-ah-1", text: "给 iOS 客户端补上待办页", createdAtMs: at(7_200)),
                TodoItem(id: "todo-ah-2", text: "把 todo-sync 的延迟说明写进 wiki", createdAtMs: at(5_400), agent: "claude"),
                TodoItem(
                    id: "todo-ah-3", text: "Check that the sync helper survives a logout and a fast user switch", createdAtMs: at(3_600),
                    att: 1
                ),
                TodoItem(id: "todo-ah-4", text: "修复深色模式下设置页的分隔线颜色", createdAtMs: at(1_800), auto: true, by: "Office iMac"),
            ],
            history: [
                TodoDoneItem(id: "todo-ah-d1", text: "整理 0.9.3 的发布说明", createdAtMs: at(90_000), doneAtMs: at(7_000)),
                TodoDoneItem(
                    id: "todo-ah-d2", text: "为 helper 增加崩溃恢复", createdAtMs: at(100_000), doneAtMs: at(40_000), doneOn: "Office iMac"
                ),
                TodoDoneItem(id: "todo-ah-d3", text: "重命名 sync 状态字段", createdAtMs: at(200_000), doneAtMs: at(90_000)),
            ]
        )
        add(
            homeMacId,
            TodoProjectRow(
                id: todoBillingProject, name: "Billing Service", repo: "github.com/acme/billing", scope: "synced", section: "synced",
                agent: "idle", macs: ["Home MacBook Pro"]
            ),
            pending: [
                TodoItem(id: "todo-bs-1", text: "Backfill the invoices that were issued during the outage", createdAtMs: at(20_000)),
                TodoItem(id: "todo-bs-2", text: "对账脚本加上幂等保护", createdAtMs: at(10_000), auto: true),
            ],
            history: [TodoDoneItem(id: "todo-bs-d1", text: "升级 Stripe SDK", createdAtMs: at(300_000), doneAtMs: at(250_000))]
        )
        add(
            homeMacId,
            TodoProjectRow(
                id: todoBlogProject, name: "blog", repo: "github.com/naituw/blog", scope: "synced", section: "synced",
                macs: ["Home MacBook Pro"]
            ),
            pending: [TodoItem(id: "todo-bl-1", text: "写一篇关于 CloudKit 同步的文章", createdAtMs: at(500_000))]
        )
        add(
            homeMacId,
            TodoProjectRow(id: todoNotesProject, name: "notes", scope: "cloudOnly", section: "synced"),
            pending: [
                TodoItem(id: "todo-nt-1", text: "整理读书笔记", createdAtMs: at(900_000), by: "Office iMac"),
                TodoItem(id: "todo-nt-2", text: "Reply to the conference organisers", createdAtMs: at(800_000), by: "Office iMac"),
            ],
            canAuto: false
        )
        add(
            homeMacId,
            TodoProjectRow(
                id: todoDotfilesProject, name: "dotfiles", repo: "github.com/naituw/dotfiles", scope: "synced", section: "empty",
                macs: ["Home MacBook Pro"]
            ),
            history: [
                TodoDoneItem(id: "todo-df-d1", text: "迁移到 mise", createdAtMs: at(700_000), doneAtMs: at(600_000)),
                TodoDoneItem(id: "todo-df-d2", text: "清理旧的 brew 包", createdAtMs: at(720_000), doneAtMs: at(610_000)),
            ]
        )
        add(
            homeMacId,
            TodoProjectRow(id: todoScratchProject, name: "scratch", scope: "localOnly", section: "localOnly"),
            pending: [TodoItem(id: "todo-sc-1", text: "试一下新的 MQTT 客户端", createdAtMs: at(30_000))]
        )

        // Office iMac: its own todos only.
        add(
            officeMacId,
            TodoProjectRow(id: todoOfficeWorkProject, name: "AskHuman", hint: "work", scope: "local", section: "withTodos", agent: "idle"),
            pending: [
                TodoItem(id: "todo-ow-1", text: "Profile the daemon start-up on the slow disk", createdAtMs: at(40_000)),
                TodoItem(id: "todo-ow-2", text: "给 CLI 加一个 --json 输出", createdAtMs: at(20_000)),
            ],
            history: [TodoDoneItem(id: "todo-ow-d1", text: "修复 Windows 下的路径分隔符", createdAtMs: at(400_000), doneAtMs: at(300_000))]
        )
        add(
            officeMacId,
            TodoProjectRow(id: todoOfficePlayProject, name: "AskHuman", hint: "playground", scope: "local", section: "withTodos"),
            pending: [TodoItem(id: "todo-op-1", text: "Try the new streaming parser", createdAtMs: at(60_000))]
        )
        add(
            officeMacId,
            TodoProjectRow(id: todoOfficeSandboxProject, name: "sandbox", scope: "local", section: "recent"),
            canAuto: true
        )

        // Office iMac once it syncs too: the shared repository is the one Home has (the phone takes iCloud's
        // projects from the Mac that serves it and only adds what no other Mac has), plus what only it has.
        syncedRows[officeMacId] = [
            TodoProjectRow(
                id: todoOfficeWorkProject, name: "AskHuman", repo: "github.com/naituw/askhuman", pending: 2, history: 1,
                scope: "synced", section: "synced", macs: ["Home MacBook Pro", "Office iMac"]
            ),
            list(
                officeMacId,
                TodoProjectRow(id: todoOfficeIntranetProject, name: "intranet", scope: "localOnly", section: "localOnly"),
                pending: [TodoItem(id: "todo-in-1", text: "Move the wiki off the old server", createdAtMs: at(50_000))]
            ),
            TodoProjectRow(id: todoOfficeSandboxProject, name: "sandbox", scope: "localOnly", section: "empty"),
        ]
        return TodoSample(rows: rows, syncedRows: syncedRows, lists: lists)
    }

    /// The `problem` that stands for a pull running right now (`syncing: true`) rather than for a pause reason.
    public static let todoSyncRunning = "running"

    /// What the Home MacBook Pro says about its iCloud sync.
    public static func todoSync(now: Date = Date(), problem: String? = nil) -> TodoSyncInfo {
        let running = problem == todoSyncRunning
        return TodoSyncInfo(
            syncing: running, paused: running ? nil : problem, helper: "running", unsynced: 0,
            lastPullMs: UInt64(now.addingTimeInterval(-40).timeIntervalSince1970 * 1000),
            lastPushMs: UInt64(now.addingTimeInterval(-90).timeIntervalSince1970 * 1000),
            devices: [
                TodoSyncDevice(name: "Home MacBook Pro", lastSyncMs: UInt64(now.addingTimeInterval(-40).timeIntervalSince1970 * 1000), thisMac: true),
                TodoSyncDevice(name: "Office iMac", lastSyncMs: UInt64(now.addingTimeInterval(-300).timeIntervalSince1970 * 1000), thisMac: false),
            ]
        )
    }
}
