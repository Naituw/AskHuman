'use strict';

// Claude Code (claude CLI) profile.
//
// 子进程注入的会话 ID env：CLAUDE_CODE_SESSION_ID（与 hook JSON 的 session_id 一致）。
// 进程名：以 `claude` 启动时 comm 即 "claude"（亦可能是解析后的版本化路径，含 "claude"）。
module.exports = {
  name: 'claude',
  sessionIdEnvVar: 'CLAUDE_CODE_SESSION_ID',
  envKeys: [
    'CLAUDECODE',
    'CLAUDE_CODE_SESSION_ID',
    'CLAUDE_CODE_CHILD_SESSION',
    'CLAUDE_PROJECT_DIR',
    'CLAUDE_CODE_ENTRYPOINT',
    'CLAUDE_CODE_REMOTE',
    'CLAUDE_CONFIG_DIR',
    'CLAUDE_PLUGIN_ROOT',
    'CLAUDE_ENV_FILE',
  ],
  processTokens: ['claude'],
  commandTokens: [],
  sessionIdJsonFields: ['session_id'],
};
