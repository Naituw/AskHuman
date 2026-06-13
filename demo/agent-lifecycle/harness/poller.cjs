'use strict';

// Process-liveness poller (the "level backbone" of the 3-layer model), per agent.
//
// 运行：node poller.cjs <agent> [intervalMs=1000]
// 周期性读取 logs/<agent>/pid.json 里的会话进程 pid，对其做 kill -0 探活，
// 记录「存活 <-> 死亡」跳变（以及 pid 变更=换了会话）到 logs/<agent>/poller.jsonl 与 stdout。
//
// 设计要点（对照需求 doc §4.8 / §5）：进程存活是不会漏的「电平信号」——
// 无论是正常退出、Ctrl-C、kill -9 还是直接关窗口，进程没了轮询都能发现，
// 这正是用来兜「SessionEnd 事件可能丢」（Codex 干脆没有 SessionEnd）的底。

const C = require('./common.cjs');

const agent = process.argv[2] || 'claude';
C.loadProfile(agent); // 校验 agent 名
const intervalMs = Math.max(200, Number(process.argv[3]) || 1000);

let curPid = null; // 当前正在守的 pid
let lastState = null; // 'alive' | 'dead' | null
let ticks = 0;

function log(kind, extra) {
  const rec = { ts: C.nowIso(), epoch_ms: Date.now(), agent, kind, pid: curPid, ...extra };
  C.appendJsonl(agent, 'poller.jsonl', rec);
  const tail = Object.entries(extra || {})
    .map(([k, v]) => `${k}=${v}`)
    .join(' ');
  process.stdout.write(`[${rec.ts}] ${kind} pid=${curPid ?? '-'} ${tail}\n`);
}

function tick() {
  ticks += 1;
  const info = C.readPidFile(agent);
  const newPid = info ? Number(info.pid) : null;

  // pid 文件出现 / 变更 → 重新 arm
  if (newPid !== curPid) {
    const prev = curPid;
    curPid = newPid;
    lastState = null;
    if (curPid) {
      log('arm', {
        prev_pid: prev ?? '-',
        session_id: (info && info.session_id) || '-',
        source: (info && info.source) || '-',
        comm: (info && info.comm) || '-',
      });
    } else {
      log('disarm', { reason: 'pidfile-gone-or-empty' });
    }
  }

  if (!curPid) {
    if (ticks % 10 === 0) log('idle', { note: `waiting for logs/${agent}/pid.json` });
    return;
  }

  const state = C.probeAlive(curPid);
  if (state !== lastState) {
    if (lastState === 'alive' && state === 'dead') {
      log('DEAD', { note: 'session process exited (level signal -> disarm)' });
    } else if (state === 'alive') {
      log('LIVE', {});
    } else if (state === 'dead') {
      log('dead', {});
    } else {
      log('unknown', {});
    }
    lastState = state;
  } else if (ticks % 15 === 0) {
    log('heartbeat', { state });
  }
}

C.ensureLogs(agent);
process.stdout.write(
  `poller started: agent=${agent} interval=${intervalMs}ms, watching ${C.pidFile(agent)}\n`
);
log('start', { interval_ms: intervalMs });
tick();
setInterval(tick, intervalMs);
