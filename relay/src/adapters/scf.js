'use strict';

// Tencent SCF "Web function" adapter: an HTTP server on 0.0.0.0:9000, configuration from
// environment variables, and APNs over HTTP/2. All protocol logic lives in ../core.js.

const http = require('node:http');
const http2 = require('node:http2');
const core = require('../core');

const APNS_ORIGINS = {
  prod: 'https://api.push.apple.com',
  dev: 'https://api.sandbox.push.apple.com',
};
const MAX_BODY_BYTES = 16 * 1024;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Sends one APNs request. HTTP/2 sessions are reused inside a warm instance and rebuilt after a
// GOAWAY / reset; a failed attempt on a stale session is retried once on a fresh one.
function createApnsSender({ origins = APNS_ORIGINS, timeoutMs = 8000 } = {}) {
  const sessions = new Map();

  function forget(env, s) {
    if (sessions.get(env) === s) sessions.delete(env);
  }

  function sessionFor(env) {
    let s = sessions.get(env);
    if (s && !s.closed && !s.destroyed) return s;
    s = http2.connect(origins[env]);
    s.on('error', () => forget(env, s));
    s.on('close', () => forget(env, s));
    s.on('goaway', () => {
      forget(env, s);
      try {
        s.close();
      } catch {
        /* already closed */
      }
    });
    sessions.set(env, s);
    return s;
  }

  function once({ env, path, headers, body }) {
    return new Promise((resolve, reject) => {
      let s;
      try {
        s = sessionFor(env);
      } catch (e) {
        reject(e);
        return;
      }
      const req = s.request({ ':method': 'POST', ':path': path, ...headers });
      const chunks = [];
      let status = 0;
      let apnsId = null;
      req.setTimeout(timeoutMs, () => {
        req.close(http2.constants.NGHTTP2_CANCEL);
        reject(new Error('apns timeout'));
      });
      req.on('response', (h) => {
        status = h[':status'];
        apnsId = h['apns-id'] || null;
      });
      req.on('data', (c) => chunks.push(c));
      req.on('end', () => {
        let reason = null;
        if (chunks.length) {
          try {
            reason = JSON.parse(Buffer.concat(chunks).toString('utf8')).reason ?? null;
          } catch {
            reason = null;
          }
        }
        resolve({ status, reason, apnsId });
      });
      req.on('error', reject);
      req.end(body);
    });
  }

  async function send(args) {
    try {
      return await once(args);
    } catch {
      sessions.delete(args.env);
      return once(args);
    }
  }
  send.close = () => {
    for (const s of sessions.values()) {
      try {
        s.destroy();
      } catch {
        /* already destroyed */
      }
    }
    sessions.clear();
  };
  return send;
}

function readJson(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    let tooLarge = false;
    req.on('data', (c) => {
      size += c.length;
      if (size > MAX_BODY_BYTES) {
        // Keep draining (without buffering) so the 413 answer can still be delivered.
        tooLarge = true;
        chunks.length = 0;
        return;
      }
      if (!tooLarge) chunks.push(c);
    });
    req.on('end', () => {
      if (tooLarge) {
        reject(new core.RelayError(413, 'too_large'));
        return;
      }
      if (chunks.length === 0) {
        resolve(null);
        return;
      }
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString('utf8')));
      } catch {
        reject(new core.RelayError(400, 'bad_request', 'json'));
      }
    });
    req.on('error', reject);
  });
}

function createServer({
  env = process.env,
  now = () => Date.now(),
  sendApns = createApnsSender(),
  retryDelayMs = 1000,
  log = (line) => process.stderr.write(`${line}\n`),
} = {}) {
  const cfg = core.parseConfig(env);
  const sign = cfg.apns ? core.makeJwtSigner(cfg.apns) : null;
  const pushLimiter = new core.TokenBuckets({ capacity: 10, refillPerSec: 2 });
  const registerLimiter = new core.TokenBuckets({ capacity: 30, refillPerSec: 0.5 });

  async function handlePush(body) {
    const nowMs = now();
    const nowSecs = Math.floor(nowMs / 1000);
    const apnsReq = core.buildPush(body, cfg, nowSecs);
    if (!pushLimiter.allow(apnsReq.rid, nowMs)) throw new core.RelayError(429, 'rate_limited');
    const args = {
      env: apnsReq.env,
      path: apnsReq.path,
      headers: { ...apnsReq.headers, authorization: `bearer ${sign(nowSecs)}` },
      body: apnsReq.body,
    };
    let res;
    try {
      res = await sendApns(args);
      if (res.status === 429 && res.reason === 'TooManyProviderTokenUpdates') {
        await sleep(retryDelayMs);
        res = await sendApns(args);
      }
    } catch {
      return { status: 502, body: { error: 'apns_error', status: 0, reason: 'network' } };
    }
    return core.mapApnsResult(res);
  }

  async function route(method, pathname, req) {
    const at = pathname.indexOf('/v1/');
    const path = at >= 0 ? pathname.slice(at) : pathname;
    if (method === 'GET' && path === '/v1/health') {
      return { status: 200, body: { ok: true, v: 1, apns: Boolean(cfg.apns && cfg.handleKey) } };
    }
    if (method === 'POST' && path === '/v1/register') {
      if (!registerLimiter.allow('all', now())) throw new core.RelayError(429, 'rate_limited');
      const result = core.register(await readJson(req), cfg, Math.floor(now() / 1000));
      return { status: 200, body: result };
    }
    if (method === 'POST' && path === '/v1/push') {
      return handlePush(await readJson(req));
    }
    throw new core.RelayError(404, 'not_found');
  }

  const server = http.createServer(async (req, res) => {
    const started = now();
    let out;
    try {
      const { pathname } = new URL(req.url || '/', 'http://relay.local');
      out = await route(req.method || 'GET', pathname, req);
    } catch (e) {
      if (e instanceof core.RelayError) {
        out = { status: e.status, body: { error: e.code } };
      } else {
        out = { status: 500, body: { error: 'internal' } };
      }
    }
    const payload = JSON.stringify(out.body);
    res.writeHead(out.status, {
      'content-type': 'application/json',
      'content-length': Buffer.byteLength(payload),
      'cache-control': 'no-store',
    });
    res.end(payload);
    // Log only the method, status and timing: never handles, device tokens or payloads.
    log(`${req.method} ${out.status} ${now() - started}ms`);
  });
  server.on('close', () => sendApns.close?.());
  return server;
}

if (require.main === module) {
  const port = Number(process.env.PORT || 9000);
  const server = createServer();
  server.listen(port, '0.0.0.0', () => {
    process.stderr.write(`askhuman relay listening on ${port}\n`);
  });
  process.on('SIGTERM', () => server.close(() => process.exit(0)));
}

module.exports = { createApnsSender, createServer };
