// Mock-only backend for the todos-window preview (src/prototype/todos-window.html). The real
// `TodosView` runs unchanged in a plain browser: Tauri's IPC and event bus are replaced by
// `mockIPC`, so the sidebar, header, lists and dialogs are the product components. Used to look at
// layouts the unit tests cannot judge (spacing, narrow drawer, dark mode, large data sets).
//
//   ?data=real|stress|empty|dups   data set (default real)
//   ?theme=light|dark              (default light)
//   ?platform=mac|win              title-bar spacing (default mac)
//   ?project=<key>                 preselect, like the tray / agent-card entries
//   ?sync=none|off|ok|first|syncing|offline|account|zone|quota|helper|problem|error
//                                  the sidebar's sync line (default: off on mac, none on win)
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type {
  TodoDoneEntry,
  TodoEntry,
  TodoGroup,
  TodoProjectInfo,
  TodoSyncStatus,
  TodosInit,
} from "../lib/types";

export type DataSet = "real" | "stress" | "empty" | "dups";
export type SyncMock =
  | "none"
  | "off"
  | "ok"
  | "first"
  | "syncing"
  | "offline"
  | "account"
  | "zone"
  | "quota"
  | "helper"
  | "problem"
  | "error";
const SYNC_MOCKS: SyncMock[] = [
  "none", "off", "ok", "first", "syncing", "offline", "account", "zone", "quota", "helper", "problem", "error",
];

export interface PreviewParams {
  data: DataSet;
  theme: "light" | "dark";
  platform: "mac" | "win";
  sync: SyncMock;
}

export function readParams(search: string = location.search): PreviewParams {
  const q = new URLSearchParams(search);
  const data = q.get("data");
  const platform = q.get("platform") === "win" ? "win" : "mac";
  const sync = q.get("sync") as SyncMock | null;
  return {
    data: data === "stress" || data === "empty" || data === "dups" ? data : "real",
    theme: q.get("theme") === "dark" ? "dark" : "light",
    platform,
    sync: sync && SYNC_MOCKS.includes(sync) ? sync : platform === "mac" ? "off" : "none",
  };
}

/** The backend's `todo_sync_status` for the chosen scenario. */
export function syncStatus(kind: SyncMock): TodoSyncStatus {
  const now = Date.now();
  const running = { state: "running", version: "1", environment: "production", problem: null, installed: true } as const;
  const base: TodoSyncStatus = {
    supported: kind !== "none",
    enabled: kind !== "none" && kind !== "off",
    syncing: false,
    deviceName: "Mac Studio",
    paused: null,
    lastPullOkMs: now - 40_000,
    lastPushOkMs: now - 100_000,
    unsynced: 0,
    todos: 12,
    history: 14,
    devices: [
      { name: "Mac Studio", lastSyncMs: now - 40_000, thisMac: true },
      { name: "MacBook Pro", lastSyncMs: now - 6 * MINUTE, thisMac: false },
    ],
    helper: { ...running },
    lastTick: null,
    newerState: false,
  };
  switch (kind) {
    case "first":
      return { ...base, lastPullOkMs: 0, lastPushOkMs: 0 };
    case "syncing":
      return { ...base, syncing: true };
    case "offline":
      return {
        ...base,
        unsynced: 3,
        lastTick: { atMs: now, reason: "poll", outcome: "offline", fetched: 0, merged: 0, pushed: 0 },
      };
    case "account":
      return { ...base, paused: "accountChanged" };
    case "zone":
      return { ...base, paused: "zoneMissing" };
    case "quota":
      return { ...base, paused: "quota" };
    case "helper":
      return { ...base, helper: { state: "notRunning", version: null, environment: null, problem: null, installed: true } };
    case "problem":
      return { ...base, helper: { ...running, problem: "missingEntitlement" } };
    case "error":
      return {
        ...base,
        lastTick: { atMs: now, reason: "poll", outcome: "error", message: "unexpected reply", fetched: 0, merged: 0, pushed: 0 },
      };
    default:
      return base;
  }
}

const HOME = "/Users/emily";
const MINUTE = 60_000;

interface Store {
  /** Pending todos per project key, in queue order. */
  todos: Map<string, TodoEntry[]>;
  history: Map<string, TodoDoneEntry[]>;
  /** Projects that show up without todos (recent workspaces). */
  recent: string[];
}

function todo(id: string, text: string, minutesAgo: number, agent?: string, auto = false): TodoEntry {
  return {
    id,
    text,
    createdAtMs: Date.now() - minutesAgo * MINUTE,
    agentKind: agent ?? null,
    ...(auto ? { auto } : {}),
  };
}

function done(id: string, text: string, minutesAgo: number): TodoDoneEntry {
  return {
    id,
    text,
    createdAtMs: Date.now() - (minutesAgo + 30) * MINUTE,
    agentKind: null,
    doneAtMs: Date.now() - minutesAgo * MINUTE,
  };
}

function realStore(): Store {
  const hil = `${HOME}/Developer/HumanInLoop`;
  const todos = new Map<string, TodoEntry[]>([
    [
      hil,
      [
        todo("h1", "待办窗口：侧栏显示每个项目在本机的目录数", 12, "cursor"),
        todo("h2", "排查 Dev Instance purge 之后残留的 launchd 条目", 70, "claude", true),
        todo("h3", "给 iOS 端加「查看待办」只读页", 60 * 26, "codex"),
        todo("h4", "把 docs/PROGRESS.md 里已完成的条目清掉", 60 * 24 * 3, "cursor"),
      ],
    ],
    [
      `${HOME}/Developer/AskHuman-iOS`,
      [
        todo("i1", "适配 iOS 27 的 Live Activity 新接口", 120),
        todo("i2", "推送时机设置页加上「Mac 已空闲 N 分钟」", 200, "claude"),
        todo("i3", "修复通知扩展解密失败时的兜底文案", 60 * 30),
      ],
    ],
    [`${HOME}/dotfiles`, [todo("d1", "zsh 启动时间优化", 60 * 25)]],
    [`${HOME}/Scratch/notes`, [todo("n1", "整理本周读书笔记", 60 * 24 * 3)]],
  ]);
  const history = new Map<string, TodoDoneEntry[]>([
    [
      hil,
      [
        done("x1", "为 todos.json 增加同步元数据（sidecar）", 60 * 24),
        done("x2", "重构设置页的 iOS 通道卡片", 60 * 24 * 3),
      ],
    ],
  ]);
  return {
    todos,
    history,
    recent: [`${HOME}/Developer/blog`, `${HOME}/Developer/side-project-x`, `${HOME}/Developer/tauri-playground`],
  };
}

function dupsStore(): Store {
  const store = realStore();
  store.todos.set(`${HOME}/Work/AskHuman`, [todo("w1", "Work 目录里的同名项目", 15)]);
  store.todos.set(`${HOME}/Developer/AskHuman`, [todo("w2", "Developer 目录里的同名项目", 30), todo("w3", "第二条", 45)]);
  store.todos.set(`${HOME}/Archive/2024/app`, [todo("w4", "更深一层才能区分", 5)]);
  store.todos.set(`${HOME}/Developer/2025/app`, [todo("w5", "另一个 app", 6)]);
  return store;
}

function stressStore(): Store {
  const todos = new Map<string, TodoEntry[]>();
  const history = new Map<string, TodoDoneEntry[]>();
  let n = 0;
  for (let p = 0; p < 200; p += 1) {
    const key = `${HOME}/Developer/project-${String(p).padStart(3, "0")}`;
    const count = p < 20 ? 50 : p < 60 ? 10 : 0;
    const list: TodoEntry[] = [];
    for (let i = 0; i < count; i += 1) {
      n += 1;
      list.push(todo(`s${n}`, `压力数据 ${p}-${i}：一条比较长的待办，用来观察换行与两行截断的效果`, n % 5000));
    }
    if (list.length) todos.set(key, list);
    if (p < 40) {
      history.set(
        key,
        Array.from({ length: 50 }, (_, i) => done(`sh${p}-${i}`, `已执行 ${p}-${i}`, i * 7 + 1))
      );
    }
  }
  return { todos, history, recent: Array.from({ length: 50 }, (_, i) => `${HOME}/Recent/recent-${i}`) };
}

function makeStore(data: DataSet): Store {
  if (data === "stress") return stressStore();
  if (data === "dups") return dupsStore();
  if (data === "empty") return { todos: new Map(), history: new Map(), recent: [] };
  return realStore();
}

function projectsOf(store: Store): TodoProjectInfo[] {
  const base = (key: string) => key.split("/").filter(Boolean).pop() ?? key;
  const withTodos = [...store.todos.entries()]
    .filter(([, list]) => list.length > 0)
    .map(([key, list]): TodoProjectInfo => ({ key, name: base(key), count: list.length, section: "withTodos" }));
  const recent = store.recent
    .filter((key) => !withTodos.some((p) => p.key === key))
    .map((key): TodoProjectInfo => ({ key, name: base(key), count: 0, section: "recent" }));
  return [...withTodos, ...recent];
}

export function installMock(params: PreviewParams): void {
  const store = makeStore(params.data);
  let seq = 1000;
  const list = (project: string) => store.todos.get(project) ?? [];
  const changed = () => void emit("todos-updated");

  void mockWindows("todos");
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      const project = typeof args.project === "string" ? args.project : "";
      switch (cmd) {
        case "todos_init":
          return {
            theme: params.theme,
            lang: "zh",
            popupSubmitKey: "cmdEnter",
            newTaskSupported: true,
            perf: false,
            home: HOME,
          } satisfies TodosInit;
        case "todo_sync_status":
        case "todo_sync_resume":
        case "todo_sync_now":
          return syncStatus(params.sync);
        case "todo_sync_history_count":
          // Synced projects keep more history in the cloud than this Mac shows.
          return params.sync === "none" || params.sync === "off"
            ? null
            : (store.history.get(project)?.length ?? 0) + 12;
        case "todos_projects":
        case "todos_projects_enriched":
          return projectsOf(store);
        case "todos_list":
          return list(project);
        case "todos_history":
          return [...(store.history.get(project) ?? [])].reverse();
        case "todos_all_pending":
          return [...store.todos.entries()]
            .filter(([, entries]) => entries.length > 0)
            .map(([key, entries]): TodoGroup => ({ key, entries }));
        case "todos_add": {
          seq += 1;
          const entry = todo(`new${seq}`, String(args.text ?? ""), 0, undefined, args.auto === true);
          store.todos.set(project, [...list(project), entry]);
          changed();
          return entry;
        }
        case "todos_complete": {
          const id = String(args.id ?? "");
          const hit = list(project).find((e) => e.id === id);
          store.todos.set(project, list(project).filter((e) => e.id !== id));
          if (hit) {
            store.history.set(project, [...(store.history.get(project) ?? []), done(id, hit.text, 0)]);
          }
          changed();
          return true;
        }
        case "todos_remove": {
          const id = String(args.id ?? "");
          store.todos.set(project, list(project).filter((e) => e.id !== id));
          changed();
          return true;
        }
        case "todos_clear":
          store.todos.set(project, []);
          changed();
          return 0;
        case "todos_history_clear":
          store.history.set(project, []);
          changed();
          return 0;
        case "todos_restore": {
          const id = String(args.id ?? "");
          const hit = (store.history.get(project) ?? []).find((e) => e.id === id);
          if (hit) {
            store.history.set(project, (store.history.get(project) ?? []).filter((e) => e.id !== id));
            store.todos.set(project, [...list(project), todo(id, hit.text, 0)]);
          }
          changed();
          return true;
        }
        case "todos_reorder": {
          const ids = (args.ids as string[] | undefined) ?? [];
          const byId = new Map(list(project).map((e) => [e.id, e]));
          store.todos.set(project, ids.map((id) => byId.get(id)).filter((e): e is TodoEntry => !!e));
          return true;
        }
        case "todos_set_auto": {
          const id = String(args.id ?? "");
          store.todos.set(
            project,
            list(project).map((e) => (e.id === id ? { ...e, auto: args.auto === true } : e))
          );
          changed();
          return true;
        }
        default:
          // Everything else (window plumbing, attachment thumbnails, ...) is irrelevant here.
          return null;
      }
    },
    { shouldMockEvents: true }
  );
}
