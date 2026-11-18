// Mock-only backend for the "complete / delete / add on every todo list" design prototype
// (docs/specs/todo-list-actions.md). Like replyMenuMock.ts it lets the real `PopupView` run in a plain browser; the
// project's todos come from here, so the real todo section, the What's-next options and the Stop card all list them.
// What the prototype adds (hover actions, the add row, the completing / confirm states) is injected into that DOM by
// TodoActionsFrame.vue. Nothing here ships.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { AskRequest, PopupInit, TodoEntry } from "../lib/types";
import { PROJECT, REQUEST_ID } from "./replyMenuMock";

export type Scene =
  | "ask"
  | "ask-states"
  | "ask-add"
  | "ask-empty"
  | "next"
  | "next-states"
  | "next-add"
  | "next-empty"
  | "stop-now"
  | "stop";

const SCENES: readonly Scene[] = [
  "ask",
  "ask-states",
  "ask-add",
  "ask-empty",
  "next",
  "next-states",
  "next-add",
  "next-empty",
  "stop-now",
  "stop",
];

export interface TodoActionParams {
  scene: Scene;
  theme: "light" | "dark";
  width: number;
  height: number;
  /** Row (0-based, counting the todo rows only) drawn as hovered; -1 for none. */
  hover: number;
  /** Scroll the content to its end, so that a short window shows the lists. */
  scroll: boolean;
}

export function readParams(search: string = location.search): TodoActionParams {
  const q = new URLSearchParams(search);
  const num = (key: string, fallback: number) => {
    const value = Number(q.get(key));
    return Number.isFinite(value) && value > 0 ? value : fallback;
  };
  const scene = q.get("scene") as Scene;
  return {
    scene: SCENES.includes(scene) ? scene : "ask",
    theme: q.get("theme") === "dark" ? "dark" : "light",
    width: num("w", 560),
    height: num("h", 700),
    hover: q.has("hover") ? Number(q.get("hover")) : -1,
    scroll: q.get("scroll") === "end",
  };
}

const TEXTS = [
  "补全 Agents 列表的空态",
  "实现通知扩展的解密",
  "整理 Markdown 渲染的快照测试",
  "把弹窗里的待办面板同步到手机，选中后随回答一起发送",
  "设置页增加离开模式的说明文案",
  "排查 Windows 下托盘图标偏移",
];

function todoEntries(): TodoEntry[] {
  return TEXTS.map((text, i) => ({
    id: `td-${i + 1}`,
    text,
    createdAtMs: Date.now() - (i + 1) * 3_600_000,
    attachments:
      i === 3
        ? [{ id: "att-1", name: "layout.png", size: 48_000, isImage: true, sourcePath: "/tmp/layout.png", path: "/tmp/layout.png", storage: "managed" as const, available: true }]
        : [],
  }));
}

const OPTION = (text: string, recommended = false) => ({ text, recommended });

/**
 * The Stop confirmation card. `live` is the redesigned one: its todos are the project's live queue exactly like
 * What's next (so the real popup lists them with the TODO badge and refreshes them), "continue" and "end" are the
 * static options. Otherwise it is the card as the product builds it today: frozen todo chips, "continue", "end".
 */
function stopRequest(id: string, todos: TodoEntry[], live: boolean): AskRequest {
  const chips = todos.slice(0, 3).map((todo) => ({
    text: `执行待办：${todo.text}`,
    recommended: false,
    todoId: todo.id,
    todoText: todo.text,
  }));
  return {
    id,
    isMarkdown: true,
    message: {
      text: ["配对接口的超时处理已经改完，单测全部通过。", "", "后续如果还要调整重试策略，可以直接告诉我。"].join("\n"),
      files: [],
    },
    questions: [
      {
        message: "Agent 准备结束本轮对话，接下来怎么做？",
        predefinedOptions: live ? [OPTION("继续对话", true), OPTION("结束对话")] : [...chips, OPTION("继续对话", true), OPTION("结束对话")],
      },
    ],
    selectOnly: false,
    single: true,
    outputFormat: "json",
    quickReply: false,
    whatsNext: live,
  };
}

function askRequest(id: string, scene: Scene, todos: TodoEntry[]): AskRequest {
  if (scene.startsWith("stop")) return stopRequest(id, todos, scene === "stop");
  const next = scene.startsWith("next");
  if (next) {
    return {
      id,
      isMarkdown: true,
      message: { text: "已完成：配对流程的单元测试与端到端测试，全部通过。", files: [] },
      questions: [
        {
          message: "接下来做什么？",
          // The todos are put in by `loadTodos` (refreshWhatsNextTodos): suggestions first, the end option stays last.
          predefinedOptions: [OPTION("收尾 iOS 通知与分组的细节", true), OPTION("结束本轮")],
        },
      ],
      selectOnly: false,
      single: false,
      outputFormat: "text",
      whatsNext: true,
      quickReply: false,
    };
  }
  return {
    id,
    isMarkdown: true,
    message: {
      text: ["接口层的重构已经完成，单测全部通过。", "", "上线前有一处行为需要你定一下。"].join("\n"),
      files: [],
    },
    questions: [
      {
        message: "请求超时之后要怎么处理？",
        predefinedOptions: [OPTION("自动重试一次"), OPTION("直接报错，并告诉用户可以重试", true), OPTION("改成可配置，默认不重试")],
      },
    ],
    selectOnly: false,
    single: false,
    outputFormat: "text",
  };
}

function initPayload(params: TodoActionParams, todos: TodoEntry[]): PopupInit {
  return {
    callerCwd: PROJECT,
    interaction: { type: "ask", request: askRequest(REQUEST_ID, params.scene, todos) },
    theme: params.theme,
    alwaysOnTop: false,
    sourceName: "Codex",
    project: PROJECT,
    projectName: "AskHuman",
    agentKind: "codex",
    agentPid: 4242,
    agentConsoleSessionId: "proto-session",
    agentSessionId: "019d3a52-7c1e-7e41-b9a8-3f6a0c2d9e10",
    agentSessionTitle: params.scene.startsWith("next") ? "收尾 iOS 通知与分组的细节" : "重构配对接口的超时处理",
    language: "zh",
    speechLanguage: "auto",
    speechShortcut: "",
    popupSubmitKey: "cmdEnter",
    verticalQuestions: true,
    perf: false,
    perfAutodismiss: false,
    warm: false,
    createdAtMs: Date.now() - 3 * 60 * 1000,
  };
}

/** Install the mock backend before the popup mounts. */
export function installMock(params: TodoActionParams): void {
  mockWindows("main");
  const initial = todoEntries();
  // The "-empty" scenes are a project that has no todo yet.
  const todos = params.scene.endsWith("-empty") ? [] : params.scene.startsWith("stop") ? initial.slice(0, 4) : initial;
  mockIPC((cmd, args) => {
    switch (cmd) {
      case "popup_init":
        return initPayload(params, todos);
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
        return todos.map((todo) => ({ ...todo }));
      case "todos_remove": {
        const id = (args as { id?: string } | undefined)?.id;
        const at = todos.findIndex((todo) => todo.id === id);
        if (at >= 0) todos.splice(at, 1);
        return true;
      }
      default:
        return null;
    }
  }, { shouldMockEvents: true });
}
