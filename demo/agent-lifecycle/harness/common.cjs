'use strict';

// Shared helpers for the agent lifecycle-signal demo (Claude / Codex / Cursor).
// Pure Node (no deps). Used by hooklog.cjs / envprobe.cjs / poller.cjs.
//
// 抽象方式：每个 Agent 家族的差异（会话 ID env 名、要收集的 env、进程识别 token、
// hook JSON 字段名）都放进 harness/profiles/<agent>.cjs；这里的逻辑全部 profile 驱动，
// 三家共用同一套进程树回溯 / kill -0 探活 / 日志落盘代码。

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

// harness/ 的上一级即 demo 根目录；脚本自定位，不写死绝对路径。
const DEMO_ROOT = path.dirname(__dirname);
const LOGS_DIR = path.join(DEMO_ROOT, 'logs');
const PROFILES_DIR = path.join(__dirname, 'profiles');

// harness 自身进程的标记：用于在进程树里把「我们自己」排除掉，
// 否则脚本路径里含 agent 关键字会误判。用扩展名无关的基名。
const SELF_MARKERS = ['hooklog', 'envprobe', 'poller'];

// 跨家族都收集的 env 前缀：即便当前 profile 是 cursor，也顺手记录是否有
// CLAUDE_*/CODEX_*，方便发现「交叉注入」。
const ALL_AGENT_ENV_PREFIXES = ['CLAUDE', 'CURSOR', 'CODEX'];

function loadProfile(agent) {
  if (!agent) throw new Error('agent name is required (claude|codex|cursor)');
  const file = path.join(PROFILES_DIR, `${agent}.cjs`);
  // eslint-disable-next-line import/no-dynamic-require, global-require
  const p = require(file);
  if (!p || !p.name) throw new Error(`invalid profile: ${file}`);
  return p;
}

function nowIso() {
  return new Date().toISOString();
}

function agentLogsDir(agent) {
  return path.join(LOGS_DIR, agent);
}

function pidFile(agent) {
  return path.join(agentLogsDir(agent), 'pid.json');
}

function ensureLogs(agent) {
  fs.mkdirSync(agentLogsDir(agent), { recursive: true });
}

function ps(pid, fmt) {
  try {
    return execFileSync('ps', ['-o', fmt, '-p', String(pid)], {
      encoding: 'utf8',
    }).trim();
  } catch {
    return '';
  }
}

function basename(p) {
  if (!p) return '';
  return String(p).split('/').pop();
}

// 从 startPid 向上回溯进程树，直到 pid<=1 或出现环。
// 每个节点含 { pid, ppid, comm(可执行路径/名), command(完整命令行) }。
function processChain(startPid) {
  const chain = [];
  const seen = new Set();
  let pid = Number(startPid);
  while (pid && pid > 1 && !seen.has(pid)) {
    seen.add(pid);
    const ppidComm = ps(pid, 'ppid=,comm=');
    if (!ppidComm) break;
    const m = ppidComm.match(/^\s*(\d+)\s+(.*)$/);
    const ppid = m ? Number(m[1]) : 0;
    const comm = m ? m[2].trim() : '';
    const command = ps(pid, 'command=');
    chain.push({ pid, ppid, comm, command });
    pid = ppid;
  }
  return chain;
}

function isSelf(entry) {
  const hay = `${entry.comm || ''} ${entry.command || ''}`;
  return SELF_MARKERS.some((mk) => hay.includes(mk));
}

// 匹配「Agent 进程」：
//   1) 可执行路径 comm 子串命中 profile.processTokens（如 claude/codex），或
//   2) argv0 的 basename 精确等于某 token（cursor 的可执行名是 "agent"），或
//   3) 完整命令行子串命中 profile.commandTokens（仅放足够特异的 token，如 "cursor-agent"）。
// **不**对完整命令行做泛 token 子串匹配——否则命令里出现的路径会把无辜 shell 误判成 agent。
function matchedAgentToken(entry, profile) {
  const comm = (entry.comm || '').toLowerCase();
  const command = (entry.command || '').toLowerCase();
  const argv0 = (entry.command || '').trim().split(/\s+/)[0] || '';
  const argv0base = basename(argv0).toLowerCase();
  for (const t of profile.processTokens || []) {
    const tok = t.toLowerCase();
    if (comm.includes(tok)) return t;
    if (argv0base === tok) return t;
  }
  for (const t of profile.commandTokens || []) {
    if (command.includes(t.toLowerCase())) return t;
  }
  return null;
}

// 在进程链里猜测「Agent 会话进程」：从自身向上，第一个命中 agent token
// 且不是 harness 自身的节点。返回 { agent, candidates }。
function guessAgentPid(chain, profile) {
  const candidates = [];
  for (const e of chain) {
    if (isSelf(e)) continue;
    const token = matchedAgentToken(e, profile);
    if (token) candidates.push({ ...e, token });
  }
  return { agent: candidates[0] || null, candidates };
}

// 与 Agent 相关的 env：当前 profile 的精确键 + 三家通用前缀（含跨家族交叉注入侦测）。
function collectAgentEnv(profile) {
  const out = {};
  const exact = new Set(profile.envKeys || []);
  if (profile.sessionIdEnvVar) exact.add(profile.sessionIdEnvVar);
  for (const k of exact) {
    if (process.env[k] !== undefined) out[k] = process.env[k];
  }
  for (const k of Object.keys(process.env)) {
    if (ALL_AGENT_ENV_PREFIXES.some((p) => k.startsWith(p))) out[k] = process.env[k];
  }
  return out;
}

function sessionIdFromEnv(profile) {
  if (profile.sessionIdEnvVar && process.env[profile.sessionIdEnvVar] !== undefined) {
    return process.env[profile.sessionIdEnvVar];
  }
  return null;
}

// hook JSON 里的会话 ID：按 profile 指定的字段名优先，再兜常见命名。
function sessionIdFromHook(hook, profile) {
  if (!hook) return null;
  const fields = [...(profile.sessionIdJsonFields || []), 'session_id', 'conversation_id'];
  for (const f of fields) {
    if (hook[f]) return hook[f];
  }
  return null;
}

// 全量 env（敏感值打码，仅保留键名与长度，方便看「有哪些键」而不泄露密钥）。
function redactedEnv() {
  const SENSITIVE = /(KEY|TOKEN|SECRET|PASSWORD|PASSWD|CREDENTIAL|AUTH)/i;
  const out = {};
  for (const [k, v] of Object.entries(process.env)) {
    out[k] = SENSITIVE.test(k) ? `<redacted len=${String(v).length}>` : v;
  }
  return out;
}

function appendJsonl(agent, file, obj) {
  ensureLogs(agent);
  fs.appendFileSync(path.join(agentLogsDir(agent), file), JSON.stringify(obj) + '\n');
}

function writeJson(agent, file, obj) {
  ensureLogs(agent);
  fs.writeFileSync(
    path.join(agentLogsDir(agent), file),
    JSON.stringify(obj, null, 2) + '\n'
  );
}

function writePidFile(agent, info) {
  ensureLogs(agent);
  fs.writeFileSync(pidFile(agent), JSON.stringify(info, null, 2) + '\n');
}

function readPidFile(agent) {
  try {
    return JSON.parse(fs.readFileSync(pidFile(agent), 'utf8'));
  } catch {
    return null;
  }
}

// kill -0 探活：返回 'alive' | 'dead' | 'unknown'
function probeAlive(pid) {
  if (!pid) return 'unknown';
  try {
    process.kill(Number(pid), 0);
    return 'alive';
  } catch (e) {
    if (e.code === 'EPERM') return 'alive'; // 存在但无权限发信号
    if (e.code === 'ESRCH') return 'dead';
    return 'unknown';
  }
}

module.exports = {
  DEMO_ROOT,
  LOGS_DIR,
  PROFILES_DIR,
  loadProfile,
  nowIso,
  agentLogsDir,
  pidFile,
  ensureLogs,
  processChain,
  basename,
  guessAgentPid,
  collectAgentEnv,
  sessionIdFromEnv,
  sessionIdFromHook,
  redactedEnv,
  appendJsonl,
  writeJson,
  writePidFile,
  readPidFile,
  probeAlive,
};
