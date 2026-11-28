#!/usr/bin/env node
// Keyless end-to-end check of the production relay entrances, the last gate of a relay release.
//
//   node probe.mjs [--url <base>]... [--n 12]
//
// For every entrance it checks that /v1/health answers 200 with {"ok":true,"apns":true}, then fires
// N simultaneous health requests on N separate connections. Every one of them must answer 200:
// with the intended 16 slots (256 MB quota / 128 MB x concurrency 8) a burst of 12 always fits,
// while a version running at concurrency 1 (2 slots) answers most of them with HTTP 432.
//
// Entrances: pass each one with --url, or leave it out to use the two production ones — the
// built-in default relay URL from src-tauri/src/config.rs (custom domain, alias `live`) and the
// explicit channels.ios.relayUrl in ~/.askhuman/config.json (the old function URL, `$DEFAULT`).
// If the old URL cannot be found the probe fails instead of skipping it.
//
// Nothing here pushes or registers anything. A one-minute burst of refusals is not billed and cannot
// ring the HTTP 432 alarm (it needs five consecutive minutes), but the burst does occupy N of the
// slots for about a second, so N is capped at the slot count the release must provide.
// Exit codes: 0 all passed, 1 a check failed, 2 bad usage.
import tls from 'node:tls';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const MAX_N = 16;
export const DEFAULT_N = 12;
const TIMEOUT_MS = 20000;
const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../../../..');

export function parseArgs(argv) {
  const out = { urls: [], n: DEFAULT_N };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--url') out.urls.push(argv[++i]);
    else if (a === '--n') out.n = Number(argv[++i]);
    else throw new Error(`unknown argument: ${a}`);
  }
  if (!Number.isInteger(out.n) || out.n < 1 || out.n > MAX_N) throw new Error(`--n must be an integer from 1 to ${MAX_N}`);
  for (const u of out.urls) if (!/^https:\/\/[^/\s]+/.test(u || '')) throw new Error(`--url must be an https URL: ${u}`);
  return out;
}

const normalize = (u) => String(u).trim().replace(/\/+$/, '');

// Pure part of entrance discovery, so it can be tested without the files.
export function entrancesFromSources({ configRs, configJson }) {
  const found = [];
  const m = /IOS_DEFAULT_RELAY_URL:\s*&str\s*=\s*"([^"]+)"/.exec(configRs || '');
  if (!m) throw new Error('cannot read IOS_DEFAULT_RELAY_URL from src-tauri/src/config.rs');
  found.push({ label: 'default relay URL (custom domain, alias live)', url: normalize(m[1]) });
  let old = null;
  try {
    old = JSON.parse(configJson).channels.ios.relayUrl;
  } catch {
    old = null;
  }
  if (!old || normalize(old) === found[0].url) {
    throw new Error('cannot find the old function URL (channels.ios.relayUrl in ~/.askhuman/config.json); pass every entrance with --url');
  }
  found.push({ label: 'old function URL ($DEFAULT)', url: normalize(old) });
  return found;
}

function discover() {
  const read = (p) => {
    try {
      return fs.readFileSync(p, 'utf8');
    } catch {
      return '';
    }
  };
  return entrancesFromSources({
    configRs: read(path.join(REPO, 'src-tauri/src/config.rs')),
    configJson: read(path.join(os.homedir(), '.askhuman/config.json')),
  });
}

// Numeric account / function ids in a host name are not worth printing.
const show = (u) => u.replace(/\d{6,}/g, '#');

export function parseHealth(res) {
  if (res.status !== 200) return { ok: false, why: `status ${res.status}${res.error ? ` (${res.error})` : ''}` };
  const m = /\{[^{}]*\}/.exec(res.body || '');
  let j = null;
  try {
    j = m ? JSON.parse(m[0]) : null;
  } catch {
    j = null;
  }
  if (!j || j.ok !== true || j.apns !== true) return { ok: false, why: `unexpected body ${(res.body || '').slice(0, 80).replace(/\s+/g, ' ')}` };
  return { ok: true, body: m[0] };
}

// Judge a burst: every one of the n requests must have answered 200.
export function judgeBurst(results, n) {
  const counts = {};
  for (const r of results) {
    const k = r.status === 0 ? (r.error ? `error:${r.error}` : 'no answer') : String(r.status);
    counts[k] = (counts[k] || 0) + 1;
  }
  const ms = results.filter((r) => r.status === 200).map((r) => r.ms).sort((a, b) => a - b);
  const ok = results.length === n && (counts['200'] || 0) === n;
  let why = '';
  if (!ok) {
    why = results.length !== n ? `only ${results.length} of ${n} requests were made` : '';
    if (counts['432']) why += `${why ? '; ' : ''}${counts['432']} refused with 432: fewer than ${n} concurrent slots behind this entrance (check per-instance concurrency, quota and alias target)`;
    const other = Object.entries(counts).filter(([k]) => k !== '200' && k !== '432');
    if (other.length) why += `${why ? '; ' : ''}other answers ${other.map(([k, v]) => `${k} x${v}`).join(', ')}`;
  }
  return { ok, counts, why, minMs: ms[0], medianMs: ms[Math.floor(ms.length / 2)], maxMs: ms[ms.length - 1] };
}

function connect(host, port) {
  return new Promise((resolve, reject) => {
    const s = tls.connect({ host, port, servername: host, ALPNProtocols: ['http/1.1'] });
    s.setNoDelay(true);
    const timer = setTimeout(() => {
      s.destroy();
      reject(new Error('connect timeout'));
    }, TIMEOUT_MS);
    s.once('secureConnect', () => {
      clearTimeout(timer);
      resolve(s);
    });
    s.once('error', (e) => {
      clearTimeout(timer);
      reject(e);
    });
  });
}

// One request on an already connected socket; the request is written before this returns.
function exchange(sock, host, route) {
  return new Promise((resolve) => {
    const t0 = performance.now();
    const chunks = [];
    let settled = false;
    const done = (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      sock.destroy();
      const raw = Buffer.concat(chunks).toString('utf8');
      const m = /^HTTP\/1\.[01] (\d{3})/.exec(raw);
      const i = raw.indexOf('\r\n\r\n');
      resolve({ status: m ? Number(m[1]) : 0, body: i >= 0 ? raw.slice(i + 4) : '', ms: Math.round(performance.now() - t0), error });
    };
    const timer = setTimeout(() => done('timeout'), TIMEOUT_MS);
    sock.on('data', (c) => chunks.push(c));
    sock.on('end', () => done());
    sock.on('close', () => done());
    sock.on('error', (e) => done(e.code || e.message));
    sock.write(`GET ${route} HTTP/1.1\r\nHost: ${host}\r\nConnection: close\r\nAccept: application/json\r\nUser-Agent: askhuman-relay-release-probe\r\n\r\n`);
  });
}

async function probeEntrance(e, n) {
  const u = new URL(e.url);
  const host = u.hostname;
  const port = Number(u.port || 443);
  const base = u.pathname.replace(/\/+$/, '');
  const route = `${base}/v1/health`;
  console.log(`== ${show(e.url)}  [${e.label}]`);
  let pass = true;

  try {
    const sock = await connect(host, port);
    const res = await exchange(sock, host, route);
    const h = parseHealth(res);
    console.log(`   health: ${h.ok ? `200 ${h.body}` : `FAIL ${h.why}`}  (${res.ms} ms)`);
    pass = pass && h.ok;
  } catch (err) {
    console.log(`   health: FAIL ${err.message}`);
    pass = false;
  }

  // Connect everything first, then write all requests in the same tick so they arrive together.
  const socks = await Promise.allSettled(Array.from({ length: n }, () => connect(host, port)));
  const results = await Promise.all(
    socks.map((s) => (s.status === 'fulfilled' ? exchange(s.value, host, route) : Promise.resolve({ status: 0, error: `connect: ${s.reason && s.reason.message}`, ms: 0 }))),
  );
  const j = judgeBurst(results, n);
  const counts = Object.entries(j.counts).map(([k, v]) => `${k} x${v}`).join(', ');
  console.log(`   burst of ${n} simultaneous requests: ${counts}${j.ok ? `  (min ${j.minMs} ms, median ${j.medianMs} ms, max ${j.maxMs} ms)` : ''}  ${j.ok ? 'PASS' : `FAIL: ${j.why}`}`);
  return pass && j.ok;
}

async function main() {
  let args;
  let entrances;
  try {
    args = parseArgs(process.argv.slice(2));
    entrances = args.urls.length ? args.urls.map((url, i) => ({ label: `--url #${i + 1}`, url: normalize(url) })) : discover();
  } catch (err) {
    console.error(`usage error: ${err.message}`);
    process.exit(err.message.startsWith('cannot') ? 1 : 2);
  }
  let all = true;
  for (let i = 0; i < entrances.length; i++) {
    if (i > 0) await new Promise((r) => setTimeout(r, 3000)); // let the first burst drain
    all = (await probeEntrance(entrances[i], args.n)) && all;
  }
  console.log(all ? 'RESULT: PASS' : 'RESULT: FAIL');
  process.exit(all ? 0 : 1);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
