// Mock-only backend for the "Note" (留言) design prototype (docs/specs/agent-notes.md). Like todoActionsMock.ts it lets
// the real `PopupView` and the real `InboxSidebar` run in a plain browser; what the prototype adds (the note tag, the
// reply field's states, the one primary button, the wake confirmation, the list's counts) is drawn over that DOM by
// NoteFrame.vue. Nothing here ships.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { AskRequest, PopupInboxRequest, PopupInit } from "../lib/types";

export type Scene =
  /** The reply is empty: the button reads "Received". */
  | "empty"
  /** Something is written: the button reads "Interject". */
  | "typed"
  /** The Agent's turn is over, a native channel can start a new one: the button reads "Wake". */
  | "idle"
  /** The same, after the click: the confirmation. */
  | "confirm"
  /** The session cannot take a reply at all: the field is off. */
  | "locked"
  /** The merged window with notes among the questions. */
  | "list"
  /** The small window: the list alone. */
  | "mini"
  /** The small window when only notes are left. */
  | "mini-notes";

export const SCENES: readonly Scene[] = ["empty", "typed", "idle", "confirm", "locked", "list", "mini", "mini-notes"];

export interface NoteParams {
  scene: Scene;
  theme: "light" | "dark";
  width: number;
  height: number;
}

export function readParams(search: string = location.search): NoteParams {
  const q = new URLSearchParams(search);
  const num = (key: string, fallback: number) => {
    const value = Number(q.get(key));
    return Number.isFinite(value) && value > 0 ? value : fallback;
  };
  const scene = q.get("scene") as Scene;
  return {
    scene: SCENES.includes(scene) ? scene : "empty",
    theme: q.get("theme") === "dark" ? "dark" : "light",
    width: num("w", 560),
    height: num("h", 640),
  };
}

export const PROJECT = "/Users/emily/Developer/AskHuman";
export const REQUEST_ID = "proto-note-1";

/** What the Agent wrote: an answer to a question the user asked earlier, nothing to decide. */
export const NOTE_TEXT = [
  "你之前问的「请求超时之后为什么没有自动重试」，我查完了：",
  "",
  "- `PairingClient` 的超时分支直接抛出 `TimeoutError`，没有经过 `retry()` 包装",
  "- 两周前为了避免重复扣费，这里的自动重试被去掉了，只在幂等的 `GET` 上保留",
  "",
  "结论：现在的行为是有意的，不是 bug。如果希望 `POST /pair` 也能重试，要先给它加幂等键。这是一个独立的改动，我先不动，这一轮收尾后再和你确认。",
].join("\n");

/** What the user typed, for the scenes that show a reply. */
export const REPLY_TEXT = "幂等键大概要改几处？如果不多，这一轮顺手做掉吧。";

// On the wire a note still carries one question with the single option "Received" (and `selectOnly`), which is what a
// build that predates notes shows and answers. A build that knows `note` hides that question, keeps the free-text field
// open (it ignores `selectOnly` for a note) and draws the one primary button instead. The prototype is such a build.
function noteRequest(id: string, text: string): AskRequest & { note: boolean } {
  return {
    id,
    isMarkdown: true,
    message: { text, files: [] },
    questions: [{ message: "", predefinedOptions: [{ text: "已收到", recommended: false }] }],
    selectOnly: false,
    single: true,
    outputFormat: "json",
    quickReply: false,
    annotatable: true,
    note: true,
  };
}

function initPayload(params: NoteParams): PopupInit {
  return {
    callerCwd: PROJECT,
    interaction: { type: "ask", request: noteRequest(REQUEST_ID, NOTE_TEXT) },
    theme: params.theme,
    alwaysOnTop: false,
    sourceName: "Codex",
    project: PROJECT,
    projectName: "AskHuman",
    agentKind: "codex",
    agentPid: 4242,
    agentConsoleSessionId: "proto-session",
    agentSessionId: "019d3a52-7c1e-7e41-b9a8-3f6a0c2d9e10",
    agentSessionTitle: "重构配对接口的超时处理",
    language: "zh",
    speechLanguage: "auto",
    speechShortcut: "",
    popupSubmitKey: "cmdEnter",
    verticalQuestions: false,
    perf: false,
    perfAutodismiss: false,
    warm: false,
    createdAtMs: Date.now() - 2 * 60 * 1000,
  };
}

type Row = {
  n: number;
  project: string;
  kind: "ask" | "note";
  agent: "codex" | "claude" | "cursor";
  session: string;
  title: string;
};

const ROWS: Row[] = [
  { n: 1, project: PROJECT, kind: "note", agent: "codex", session: "重构配对接口的超时处理", title: "你之前问的「请求超时之后为什么没有自动重试」，我查完了" },
  { n: 2, project: PROJECT, kind: "ask", agent: "claude", session: "修复登录页样式", title: "要采用哪种修复方式？" },
  { n: 3, project: PROJECT, kind: "note", agent: "cursor", session: "发布前检查清单", title: "单测补完了，另外发现两处无关的告警，已记进待办" },
  { n: 4, project: "/Users/emily/Developer/website", kind: "ask", agent: "cursor", session: "首页改版", title: "选择首页的内容布局" },
];

/** The rows of the Sidebar; `only` keeps one kind (the small window that only has notes left). */
export function sidebarRequests(only?: "note"): PopupInboxRequest[] {
  const base = Date.now() - 6 * 60 * 1000;
  return ROWS.filter((row) => !only || row.kind === only).map((row) => {
    const id = `proto-note-${row.n}`;
    const request: AskRequest =
      row.kind === "note"
        ? noteRequest(id, row.title)
        : {
            id,
            isMarkdown: true,
            message: { text: "", files: [] },
            questions: [{ message: row.title, predefinedOptions: [] }],
            selectOnly: false,
            single: false,
            outputFormat: "text",
          };
    return {
      requestId: id,
      sequence: row.n,
      interaction: { type: "ask", request },
      project: row.project,
      source: row.agent,
      lang: "zh",
      agentKind: row.agent,
      agentSessionTitle: row.session,
      createdAtMs: base + row.n * 40_000,
      // The product tells a note from a question by this; the Sidebar prints it as a tag, as it does for "permission".
      kind: row.kind as PopupInboxRequest["kind"],
    };
  });
}

/** Install the mock backend before the popup mounts. */
export function installMock(params: NoteParams): void {
  mockWindows("main");
  mockIPC((cmd) => {
    switch (cmd) {
      case "popup_init":
        return initPayload(params);
      case "popup_agent_terminal":
        return "apple-terminal";
      case "popup_agent_resolved":
        return { kind: "codex", pid: 4242, launchId: null };
      case "popup_agent_info":
        return {
          model: "gpt-5.5",
          reasoning: "high",
          context: { usedTokens: 106_000, windowTokens: 200_000, remainingPercent: 50, estimated: false },
          reportedAt: Math.floor(Date.now() / 1000) - 40,
        };
      case "popup_project_info":
        return { branch: "main", detached: false, worktree: null, changes: { total: 3, staged: 1, unstaged: 2, untracked: 0 } };
      case "todos_list":
        return [];
      case "popup_setup_hint":
        return null;
      default:
        return null;
    }
  }, { shouldMockEvents: true });
}
