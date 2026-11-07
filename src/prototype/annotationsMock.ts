// Mock backend for the "annotations in the popup" prototype. Like replyMenuMock.ts it lets the real `PopupView` run in a
// plain browser; on top of that it can build a *direct ask* (no Message, one long question), the one shape in which the
// question text is the body of the request and therefore can carry annotations.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { AskRequest, PopupInit } from "../lib/types";
import { PROJECT, REQUEST_ID, readParams as readBase, type FrameParams } from "./replyMenuMock";

export type Shape = "message" | "direct" | "three";
export interface AnnotationParams extends FrameParams {
  shape: Shape;
}

export function readParams(search: string = location.search): AnnotationParams {
  const base = readBase(search);
  const shape = new URLSearchParams(search).get("shape");
  return { ...base, shape: shape === "direct" ? "direct" : shape === "three" ? "three" : "message" };
}

const OPTION = (text: string, recommended = false) => ({ text, recommended });

function askRequest(id: string, shape: Shape): AskRequest {
  const base = { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text" as const };
  if (shape === "direct") {
    return {
      ...base,
      message: { text: "", files: [] },
      questions: [
        {
          message: [
            "迁移脚本已经跑完，结果有两点要你确认：",
            "",
            "- 旧表 `orders_v1` 里有 1,203 行没有 `user_id`，我先统一标成了 `unknown`",
            "- 新表比旧表少 3 个索引，查询在 staging 上慢了约 18%",
            "",
            "要不要现在补齐索引，再切流量？",
          ].join("\n"),
          predefinedOptions: [OPTION("补齐索引后再切流量", true), OPTION("先切流量，索引稍后补"), OPTION("先不切")],
        },
      ],
    };
  }
  if (shape === "three") {
    return {
      ...base,
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
        { message: "会话存储用哪种方案？", predefinedOptions: [OPTION("Cookie + 服务端校验", true), OPTION("独立会话表"), OPTION("先不定，我再想想")] },
        { message: "旧会话数据是否一并迁移？", predefinedOptions: [OPTION("本次一并迁移"), OPTION("以后单独处理", true)] },
        { message: "测试覆盖到什么程度？", predefinedOptions: [OPTION("只覆盖核心路径"), OPTION("含边界与回归", true)] },
      ],
    };
  }
  return {
    ...base,
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
        predefinedOptions: [OPTION("方案 1：margin 兜底", true), OPTION("方案 2：改用 grid 统一"), OPTION("先不改，只记录问题")],
      },
    ],
  };
}

function initPayload(params: AnnotationParams): PopupInit {
  return {
    callerCwd: PROJECT,
    interaction: { type: "ask", request: askRequest(REQUEST_ID, params.shape) },
    theme: params.theme,
    alwaysOnTop: false,
    sourceName: "Codex",
    project: PROJECT,
    projectName: "AskHuman",
    agentKind: "codex",
    agentPid: 4242,
    agentConsoleSessionId: "proto-session",
    agentSessionId: "019d3a52-7c1e-7e41-b9a8-3f6a0c2d9e10",
    agentSessionTitle: params.shape === "direct" ? "订单表迁移" : params.shape === "three" ? "重构登录会话存储" : "修复登录页 Safari 样式错位",
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
export function installMock(params: AnnotationParams): void {
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
}
