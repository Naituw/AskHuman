// Mock-only backend for the "Agent session title in the popup" prototype
// (docs/specs/popup-session-info.md). The real `PopupView` runs unchanged in a plain browser: Tauri's IPC and
// event bus are replaced by `mockIPC`, so the popup core, navbar, content and footer are the product components
// and not look-alikes. Variant A (the title strip) was chosen and now ships in PopupNavbar; only the rejected
// variants B-D (SessionTitleFrame.vue) are prototype code.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { AskRequest, PopupInboxRequest, PopupInit } from "../lib/types";

/**
 * The ways of showing the title that the prototype compares. `strip` is the shipped product behavior (a cell without
 * it hides the strip); the others are overlaid on the real popup and can be combined.
 */
export type Variant = "strip" | "caption" | "reply" | "inline";
export type Platform = "mac" | "win";
export interface FrameParams {
  variants: Variant[];
  title: string;
  theme: "light" | "dark";
  platform: Platform;
  /** Width of the answer pane (the window is wider by the Sidebar when it is shown). */
  width: number;
  height: number;
  sidebar: boolean;
  /** Milliseconds after which the title arrives (the product reads it asynchronously); 0 = in `popup_init`. */
  late: number;
  /** Open the product's hover card on the Agent chip (where the title shows today besides the Sidebar). */
  hover: boolean;
}

export const REQUEST_ID = "proto-request-1";
export const PROJECT = "/Users/emily/Developer/AskHuman";

/** Named samples for `?title=`; any other value is used literally. */
export const TITLE_SAMPLES: Record<string, string> = {
  short: "统一作答窗口设计",
  medium: "修复登录页 Safari 样式错位",
  long: "帮我梳理一下 Agent Session 标题在弹窗里的展示方案，并先做一个原型给我看效果，再决定怎么落地",
  english: "Refactor popup session title rendering for narrow windows",
  none: "",
};

const VARIANTS: readonly Variant[] = ["strip", "caption", "reply", "inline"];

export function readParams(search: string = location.search): FrameParams {
  const q = new URLSearchParams(search);
  const titleParam = q.get("title") ?? "medium";
  const num = (key: string, fallback: number) => {
    const value = Number(q.get(key));
    return Number.isFinite(value) && value > 0 ? value : fallback;
  };
  return {
    variants: (q.get("variant") ?? "").split(",").filter((v): v is Variant => (VARIANTS as readonly string[]).includes(v)),
    title: titleParam in TITLE_SAMPLES ? TITLE_SAMPLES[titleParam] : titleParam,
    theme: q.get("theme") === "dark" ? "dark" : "light",
    platform: q.get("platform") === "win" ? "win" : "mac",
    width: num("w", 560),
    height: num("h", 620),
    sidebar: q.get("sidebar") === "1",
    late: Math.max(0, Number(q.get("late")) || 0),
    hover: q.get("hover") === "agent",
  };
}

function askRequest(id: string): AskRequest {
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

function initPayload(params: FrameParams): PopupInit {
  return {
    callerCwd: PROJECT,
    interaction: { type: "ask", request: askRequest(REQUEST_ID) },
    theme: params.theme,
    alwaysOnTop: false,
    sourceName: "Codex",
    project: PROJECT,
    projectName: "AskHuman",
    agentKind: "codex",
    agentPid: 4242,
    agentConsoleSessionId: "proto-session",
    agentSessionId: "019d3a52-7c1e-7e41-b9a8-3f6a0c2d9e10",
    agentSessionTitle: params.late > 0 ? "" : params.title,
    language: "zh",
    speechLanguage: "auto",
    speechShortcut: "",
    popupSubmitKey: "cmdEnter",
    verticalQuestions: false,
    perf: false,
    perfAutodismiss: false,
    warm: false,
    createdAtMs: Date.now() - 3 * 60 * 1000,
  };
}

/** What the Sidebar lists in the merged-window frames: the first row is the request the main pane shows. */
export function sidebarRequests(params: FrameParams): PopupInboxRequest[] {
  const base = Date.now() - 3 * 60 * 1000;
  const row = (n: number, project: string, title: string, message: string, question: string, agent: "codex" | "claude" | "cursor"): PopupInboxRequest => ({
    requestId: `proto-request-${n}`,
    sequence: n,
    interaction: {
      type: "ask",
      request: {
        ...askRequest(`proto-request-${n}`),
        message: { text: message, files: [] },
        questions: [{ message: question, predefinedOptions: [] }],
      },
    },
    project,
    source: agent,
    lang: "zh",
    agentKind: agent,
    agentSessionTitle: title,
    createdAtMs: base + n * 20_000,
    kind: "ask",
  });
  return [
    row(1, PROJECT, params.title, "已定位到登录页在 Safari 15 下样式错位的原因", "要采用哪种修复方式？", "codex"),
    row(2, PROJECT, "迁移历史记录到 SQLite", "迁移脚本已写好", "迁移前是否需要保留旧文件？", "claude"),
    row(3, PROJECT, "发布前检查清单", "发布分支已准备好", "是否现在打 tag？", "codex"),
    row(4, "/Users/emily/Developer/website", "首页改版", "首页文案已更新", "选择首页的内容布局", "cursor"),
  ];
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
      case "popup_im_tip_visible":
        return false;
      default:
        return null;
    }
  }, { shouldMockEvents: true });
  if (params.late > 0) {
    // The product fills the title in after the first paint (`popup-session-title`), so a layout must not jump.
    window.setTimeout(() => { void emit("popup-session-title", { requestId: REQUEST_ID, title: params.title }); }, params.late);
  }
}
