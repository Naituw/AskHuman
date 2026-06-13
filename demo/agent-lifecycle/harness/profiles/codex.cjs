'use strict';

// Codex CLI profile.
//
// 源码确认（codex-rs/core/src/unified_exec/process_manager.rs + exec_env.rs）：
//   Codex 跑 shell 工具时向子进程注入 CODEX_THREAD_ID（= 线程/会话 ID），
//   即便 shell_environment_policy.include_only 也照注。故「不用 Hook 拿会话 ID」对 Codex 成立。
// hook JSON 会话字段名同 Claude：session_id（无 reason，无 SessionEnd）。
// 进程名：codex 二进制 comm 含 "codex"。
module.exports = {
  name: 'codex',
  sessionIdEnvVar: 'CODEX_THREAD_ID',
  envKeys: [
    'CODEX_THREAD_ID',
    'CODEX_CI',
    'CODEX_HOME',
    'CODEX_SANDBOX',
    'CODEX_SANDBOX_NETWORK_DISABLED',
    'CODEX_MANAGED_BY_NPM',
  ],
  processTokens: ['codex'],
  commandTokens: [],
  sessionIdJsonFields: ['session_id'],
};
