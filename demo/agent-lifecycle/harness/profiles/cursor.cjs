'use strict';

// Cursor Agent (cursor-agent CLI) profile.
//
// 实测旁证：cursor-agent 会话内子进程 ambient env 含 CURSOR_CONVERSATION_ID（=会话 ID）、
// CURSOR_AGENT=1 / CURSOR_INVOKED_AS=agent。故「不用 Hook 拿会话 ID」对 cursor-agent CLI 也成立。
// hook JSON 会话字段名：conversation_id（另有 generation_id / workspace_roots / transcript_path）。
// 进程识别坑：cursor-agent 的可执行名是 `agent`（~/.local/bin/agent … index.js），不含 "cursor-agent"
//   字样——故 processTokens 用 argv0 basename "agent"，并用特异的 commandTokens "cursor-agent"
//   对完整命令行兜底（避免漏识别，又不至于把任意 *agent 进程误判）。
module.exports = {
  name: 'cursor',
  sessionIdEnvVar: 'CURSOR_CONVERSATION_ID',
  envKeys: [
    'CURSOR_AGENT',
    'CURSOR_INVOKED_AS',
    'CURSOR_CONVERSATION_ID',
  ],
  processTokens: ['cursor-agent', 'agent'],
  commandTokens: ['cursor-agent'],
  sessionIdJsonFields: ['conversation_id'],
};
