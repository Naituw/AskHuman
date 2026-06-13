'use strict';

// Agent lifecycle-hook logger (shared across Claude / Codex / Cursor).
//
// 由各家项目级配置的 hook 调用：
//   node hooklog.cjs <agent> <EventName>
// 从 stdin 读 hook JSON，补上墙钟时间 / 自身 pid·ppid / 进程链 / 猜到的 agent 进程 pid /
// 关键 env，追加一行到 logs/<agent>/events.jsonl。
//
// 关键纪律（fail-open）：无论如何都 exit 0、stdout 不输出任何东西。
//   - UserPromptSubmit / SessionStart 的 stdout 会被当作「注入上下文」喂给模型，
//     所以这里绝不能往 stdout 写日志，否则会污染会话。

const fs = require('fs');
const C = require('./common.cjs');

function readStdin() {
  try {
    return fs.readFileSync(0, 'utf8');
  } catch {
    return '';
  }
}

function main() {
  const agent = process.argv[2] || 'claude';
  const event = process.argv[3] || 'Unknown';
  const profile = C.loadProfile(agent);
  const raw = readStdin();

  let hook = null;
  try {
    hook = raw ? JSON.parse(raw) : null;
  } catch {
    hook = { _parse_error: true, _raw: raw.slice(0, 2000) };
  }

  const chain = C.processChain(process.pid);
  const { agent: ag, candidates } = C.guessAgentPid(chain, profile);
  const env = C.collectAgentEnv(profile);
  const sessionId = C.sessionIdFromHook(hook, profile) || C.sessionIdFromEnv(profile);

  const rec = {
    ts: C.nowIso(),
    epoch_ms: Date.now(),
    agent,
    event,
    // hook JSON 里的关键字段（best-effort，跨家族尽量都记）
    json_event: hook && hook.hook_event_name,
    session_id: sessionId,
    transcript_path: hook && hook.transcript_path,
    cwd: hook && hook.cwd,
    permission_mode: hook && hook.permission_mode,
    source: hook && hook.source, // SessionStart
    reason: hook && hook.reason, // Claude SessionEnd（Codex 无）
    turn_id: hook && hook.turn_id, // Codex UserPromptSubmit
    prompt: hook && typeof hook.prompt === 'string' ? hook.prompt.slice(0, 200) : undefined,
    tool_name: hook && hook.tool_name, // Pre/PostToolUse
    stop_hook_active: hook && hook.stop_hook_active, // Stop
    // 进程视角
    hook_pid: process.pid,
    hook_ppid: process.ppid,
    agent_pid: ag ? ag.pid : null,
    agent_comm: ag ? ag.comm : null,
    agent_token: ag ? ag.token : null,
    // env 里 agent 注入的关键变量（hook 子进程通常也能拿到会话 ID env）
    env,
    // 完整进程链（便于核对 agent_pid 猜得对不对）
    chain,
    // 多个候选（理论上应只有一个 agent）
    agent_candidates: candidates.map((c) => ({ pid: c.pid, comm: c.comm, token: c.token })),
  };

  C.appendJsonl(agent, 'events.jsonl', rec);

  // 把「当前会话的 agent 进程」写入 pid 文件，供 poller 守活。仅在猜到 agent 时更新。
  if (ag && ag.pid) {
    C.writePidFile(agent, {
      pid: ag.pid,
      comm: ag.comm,
      command: ag.command,
      session_id: sessionId,
      source: `hook:${event}`,
      ts: rec.ts,
    });
  }
}

try {
  main();
} catch (e) {
  // 即使内部出错也要 fail-open，并留个痕迹。
  try {
    C.appendJsonl(process.argv[2] || 'claude', 'events.jsonl', {
      ts: C.nowIso(),
      event: process.argv[3] || 'Unknown',
      _error: String(e && e.stack ? e.stack : e),
    });
  } catch {}
}
process.exit(0);
