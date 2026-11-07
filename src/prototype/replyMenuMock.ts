// Mock-only backend for the "quick replies and away / mute directives in the popup" prototype. The real `PopupView`
// runs unchanged in a plain browser (Tauri's IPC and event bus are replaced by `mockIPC`), so the navbar, questions and
// footer are the product components; only the reply menu, the armed bar and the away banner are prototype code
// (ReplyMenuFrame.vue), teleported into the real DOM.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { AskRequest, PopupInit } from "../lib/types";

export type Scene = "idle" | "menu" | "armed-away" | "armed-mute" | "escalated";
export interface FrameParams {
  scene: Scene;
  /** Number of questions: 1 (the common case) or 3 (the vertical multi-question layout). */
  questions: 1 | 3;
  theme: "light" | "dark";
  platform: "mac" | "win";
  width: number;
  height: number;
  /** Show the merged window's "answer later" button in the footer (the default window mode has it). */
  later: boolean;
  /** Pre-select the first option of the first question, so the menu can say how many questions are still open. */
  answered: boolean;
  /** Row of the menu to draw as hovered (0-based, counting presets then directives); -1 for none. */
  hover: number;
  note: string;
  /** Away mode is already on: escalated requests carry a banner, and the menu offers to end it. */
  away: boolean;
}

export const REQUEST_ID = "proto-request-1";
export const PROJECT = "/Users/emily/Developer/AskHuman";
const SCENES: readonly Scene[] = ["idle", "menu", "armed-away", "armed-mute", "escalated"];

export function readParams(search: string = location.search): FrameParams {
  const q = new URLSearchParams(search);
  const num = (key: string, fallback: number) => {
    const value = Number(q.get(key));
    return Number.isFinite(value) && value > 0 ? value : fallback;
  };
  const scene = q.get("scene") as Scene;
  return {
    scene: SCENES.includes(scene) ? scene : "idle",
    questions: q.get("q") === "3" ? 3 : 1,
    theme: q.get("theme") === "dark" ? "dark" : "light",
    platform: q.get("platform") === "win" ? "win" : "mac",
    width: num("w", 560),
    height: num("h", 620),
    later: q.get("later") !== "0",
    answered: q.get("answered") === "1",
    hover: q.has("hover") ? Number(q.get("hover")) : -1,
    note: q.get("note") ?? "",
    away: q.get("away") === "1" || q.get("scene") === "escalated",
  };
}

function askRequest(id: string, count: 1 | 3): AskRequest {
  if (count === 1) {
    return {
      id,
      isMarkdown: true,
      message: {
        text: [
          "已定位到登录页在 Safari 15 下样式错位的原因：`.field-row` 用了 flex 的 `gap`，旧版本不支持。",
          "",
          "两种修复方式：",
          "",
          "- **方案 1：margin 兜底**。改动小，但会同时保留两套间距规则",
          "- **方案 2：改用 grid 统一**。一次到位，需要同时调整三处表单",
          "",
          "另外我顺手确认了 Chrome 与 Firefox 下表现一致，回归测试也已补充。",
        ].join("\n"),
        files: [],
      },
      questions: [
        {
          message: "要采用哪种修复方式？",
          predefinedOptions: [
            { text: "方案 1：margin 兜底", recommended: true },
            { text: "方案 2：改用 grid 统一", recommended: false },
            { text: "先不改，只记录问题", recommended: false },
          ],
        },
      ],
      selectOnly: false,
      single: false,
      outputFormat: "text",
    };
  }
  return {
    id,
    isMarkdown: true,
    message: {
      text: [
        "登录模块的会话方案已经梳理完，实现前有三个点需要你拍板。",
        "",
        "- **存储**：Cookie 方案改动小；独立会话表更利于后续做多端登录管理",
        "- **迁移**：线上还有约 1.2 万条旧会话",
        "- **测试**：核心路径已有用例，边界与回归还没补",
      ].join("\n"),
      files: [],
    },
    questions: [
      {
        message: "会话存储用哪种方案？",
        predefinedOptions: [
          { text: "Cookie + 服务端校验", recommended: true },
          { text: "独立会话表", recommended: false },
          { text: "先不定，我再想想", recommended: false },
        ],
      },
      {
        message: "旧会话数据是否一并迁移？",
        predefinedOptions: [
          { text: "本次一并迁移", recommended: false },
          { text: "以后单独处理", recommended: true },
        ],
      },
      {
        message: "测试覆盖到什么程度？",
        predefinedOptions: [
          { text: "只覆盖核心路径", recommended: false },
          { text: "含边界与回归", recommended: true },
        ],
      },
    ],
    selectOnly: false,
    single: false,
    outputFormat: "text",
  };
}

function initPayload(params: FrameParams): PopupInit {
  return {
    callerCwd: PROJECT,
    interaction: { type: "ask", request: askRequest(REQUEST_ID, params.questions) },
    theme: params.theme,
    alwaysOnTop: false,
    sourceName: "Codex",
    project: PROJECT,
    projectName: "AskHuman",
    agentKind: "codex",
    agentPid: 4242,
    agentConsoleSessionId: "proto-session",
    agentSessionId: "019d3a52-7c1e-7e41-b9a8-3f6a0c2d9e10",
    agentSessionTitle: params.questions === 1 ? "修复登录页 Safari 样式错位" : "重构登录会话存储",
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
export function installMock(params: FrameParams): void {
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
