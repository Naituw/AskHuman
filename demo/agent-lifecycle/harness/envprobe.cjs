'use strict';

// CLI-tool environment probe (the "no-hook" path), shared across agents.
//
// 让 agent 在会话里用其 shell/Bash 工具跑：
//   node /…/demo/agent-lifecycle/harness/envprobe.cjs <agent>
// 它模拟「AskHuman 被 Agent 当子进程调用」那一刻能看到什么：
//   - agent 注入的 env（会话 ID env 等）
//   - 自身进程链，以及能否向上 walk 到 agent 进程拿到其 pid
// 然后把会话进程 pid 写入 logs/<agent>/pid.json（供 poller 守活），
// 并把全量 env（敏感打码）落到 logs 里。
//
// 与 hooklog 不同：这是工具调用，stdout 会回显给 agent / 用户，所以这里故意打印可读摘要。

const C = require('./common.cjs');

function main() {
  const agent = process.argv[2] || 'claude';
  const profile = C.loadProfile(agent);

  const chain = C.processChain(process.pid);
  const { agent: ag, candidates } = C.guessAgentPid(chain, profile);
  const env = C.collectAgentEnv(profile);
  const sessionId = C.sessionIdFromEnv(profile);

  const report = {
    ts: C.nowIso(),
    epoch_ms: Date.now(),
    agent,
    self_pid: process.pid,
    self_ppid: process.ppid,
    agent_pid: ag ? ag.pid : null,
    agent_comm: ag ? ag.comm : null,
    agent_command: ag ? ag.command : null,
    agent_token: ag ? ag.token : null,
    agent_alive: ag ? C.probeAlive(ag.pid) : 'unknown',
    agent_env: env,
    session_id_env_var: profile.sessionIdEnvVar,
    session_id_from_env: sessionId,
    chain,
    agent_candidates: candidates.map((c) => ({ pid: c.pid, comm: c.comm, token: c.token })),
    full_env_redacted: C.redactedEnv(),
  };

  C.appendJsonl(agent, 'envprobe.jsonl', report);
  C.writeJson(agent, 'envprobe-latest.json', report);

  if (ag && ag.pid) {
    C.writePidFile(agent, {
      pid: ag.pid,
      comm: ag.comm,
      command: ag.command,
      session_id: sessionId,
      source: 'envprobe',
      ts: report.ts,
    });
  }

  // 可读摘要（stdout 会回显给 agent 与用户）
  const lines = [];
  lines.push(`=== ENV PROBE (no-hook path) — agent=${agent} ===`);
  lines.push(`time            : ${report.ts}`);
  lines.push(`self pid/ppid   : ${report.self_pid} / ${report.self_ppid}`);
  lines.push('');
  lines.push(`--- agent-injected env (key question: is session id here? expect ${profile.sessionIdEnvVar}) ---`);
  if (Object.keys(env).length === 0) {
    lines.push('(none found — no CLAUDE_*/CURSOR_*/CODEX_* env visible)');
  } else {
    for (const [k, v] of Object.entries(env)) lines.push(`${k} = ${v}`);
  }
  lines.push('');
  lines.push('--- agent process discovered by walking the tree ---');
  if (ag) {
    lines.push(`agent_pid       : ${ag.pid} (alive=${report.agent_alive}, matched "${ag.token}")`);
    lines.push(`agent_comm      : ${ag.comm}`);
    lines.push(`agent_command   : ${ag.command}`);
  } else {
    lines.push('(could not locate an agent process in the parent chain)');
  }
  lines.push('');
  lines.push('--- process chain (self -> ... -> root) ---');
  for (const e of chain) {
    lines.push(`  pid=${e.pid} ppid=${e.ppid} comm=${e.comm}`);
  }
  lines.push('');
  lines.push(`wrote: logs/${agent}/envprobe-latest.json , logs/${agent}/pid.json`);
  process.stdout.write(lines.join('\n') + '\n');
}

try {
  main();
} catch (e) {
  process.stdout.write('envprobe error: ' + String(e && e.stack ? e.stack : e) + '\n');
}
process.exit(0);
