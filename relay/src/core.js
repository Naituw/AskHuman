'use strict';

// Runtime-agnostic core of the AskHuman push relay.
//
// Only Node built-ins (`crypto`) are used; there is no HTTP server, no environment access and no
// network I/O here. `adapters/scf.js` wires this core to Tencent SCF (HTTP server, env vars, APNs
// over HTTP/2). Porting to another cloud only needs a new adapter.
//
// Trust model: the relay sees the device token and the opaque end-to-end encrypted envelope `ah`
// (it can never decrypt it). A Mac only holds an opaque, revocable "handle"; the APNs signing key
// never leaves the relay.

const crypto = require('node:crypto');

const HANDLE_PREFIX = 'v1.';
const HANDLE_AAD = Buffer.from('ah-relay-handle-v1');
const HANDLE_LIFETIME_SECS = 180 * 24 * 3600;
const MAX_PUSH_BYTES = 4096; // APNs payload limit for alert pushes.
const JWT_WINDOW_SECS = 1800;

// The relay builds the alert itself from this allowlist, so a Mac can never choose arbitrary
// alert text. The strings are localized on the phone (Localizable.xcstrings).
const LOC_KEYS = new Set([
  'PUSH_ASK',
  'PUSH_CONFIRM',
  'PUSH_FOLLOW_DONE',
  'PUSH_FOLLOW_WAITING',
  'PUSH_FOLLOW_FAILED',
  'PUSH_RESOLVED',
]);

class RelayError extends Error {
  constructor(status, code, detail) {
    super(code);
    this.status = status;
    this.code = code;
    this.detail = detail;
  }
}

function b64u(buf) {
  return Buffer.from(buf).toString('base64url');
}

function isB64u(s) {
  return typeof s === 'string' && /^[A-Za-z0-9_-]*$/.test(s);
}

// ---------------------------------------------------------------- config

function parseConfig(env) {
  let handleKey = null;
  if (env.RELAY_HANDLE_KEY) {
    handleKey = Buffer.from(env.RELAY_HANDLE_KEY, 'base64');
    if (handleKey.length !== 32) {
      throw new Error('RELAY_HANDLE_KEY must decode to 32 bytes');
    }
  }
  const p8 = env.APNS_KEY_P8 ? String(env.APNS_KEY_P8).replace(/\\n/g, '\n') : '';
  const apnsConfigured = Boolean(p8 && env.APNS_KEY_ID && env.APNS_TEAM_ID && env.APNS_TOPIC);
  const revoked = new Set(
    String(env.REVOKED_RIDS || '')
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean),
  );
  return {
    handleKey,
    topic: env.APNS_TOPIC || '',
    apns: apnsConfigured
      ? { p8, keyId: env.APNS_KEY_ID, teamId: env.APNS_TEAM_ID, topic: env.APNS_TOPIC }
      : null,
    revoked,
  };
}

// ---------------------------------------------------------------- handle

function encodeHandle(key, payload) {
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv('aes-256-gcm', key, iv);
  cipher.setAAD(HANDLE_AAD);
  const ct = Buffer.concat([cipher.update(JSON.stringify(payload), 'utf8'), cipher.final()]);
  return HANDLE_PREFIX + b64u(Buffer.concat([iv, ct, cipher.getAuthTag()]));
}

function decodeHandle(key, handle) {
  const bad = () => new RelayError(401, 'bad_handle');
  if (typeof handle !== 'string' || !handle.startsWith(HANDLE_PREFIX) || handle.length > 1024) {
    throw bad();
  }
  const body = handle.slice(HANDLE_PREFIX.length);
  if (!isB64u(body)) throw bad();
  const raw = Buffer.from(body, 'base64url');
  if (raw.length < 12 + 16 + 2) throw bad();
  try {
    const decipher = crypto.createDecipheriv('aes-256-gcm', key, raw.subarray(0, 12));
    decipher.setAAD(HANDLE_AAD);
    decipher.setAuthTag(raw.subarray(raw.length - 16));
    const pt = Buffer.concat([decipher.update(raw.subarray(12, raw.length - 16)), decipher.final()]);
    const payload = JSON.parse(pt.toString('utf8'));
    if (
      typeof payload.dt !== 'string' ||
      (payload.e !== 'dev' && payload.e !== 'prod') ||
      typeof payload.b !== 'string' ||
      typeof payload.x !== 'number' ||
      typeof payload.r !== 'string'
    ) {
      throw bad();
    }
    return payload;
  } catch (e) {
    if (e instanceof RelayError) throw e;
    throw bad();
  }
}

// ---------------------------------------------------------------- register

function normalizeDeviceToken(raw) {
  if (typeof raw !== 'string') return null;
  const t = raw.replace(/[\s<>]/g, '').toLowerCase();
  if (!/^[0-9a-f]+$/.test(t) || t.length < 32 || t.length > 200 || t.length % 2 !== 0) return null;
  return t;
}

function register(body, cfg, nowSecs) {
  if (!cfg.handleKey || !cfg.topic) throw new RelayError(503, 'not_configured');
  if (!body || typeof body !== 'object') throw new RelayError(400, 'bad_request', 'body');
  const dt = normalizeDeviceToken(body.deviceToken);
  if (!dt) throw new RelayError(400, 'bad_request', 'deviceToken');
  if (body.env !== 'dev' && body.env !== 'prod') throw new RelayError(400, 'bad_request', 'env');
  // Refuse other apps of the same team so the relay cannot be used as a generic APNs proxy.
  if (body.bundleId !== cfg.topic) throw new RelayError(400, 'bad_request', 'bundleId');
  const expiresAt = nowSecs + HANDLE_LIFETIME_SECS;
  const handle = encodeHandle(cfg.handleKey, {
    dt,
    e: body.env,
    b: body.bundleId,
    i: nowSecs,
    x: expiresAt,
    r: b64u(crypto.randomBytes(12)),
  });
  return { handle, expiresAt };
}

// ---------------------------------------------------------------- push

const THREAD_RE = /^[A-Za-z0-9._:\-]{1,64}$/;

function resolveHandle(handle, cfg, nowSecs) {
  if (!cfg.handleKey) throw new RelayError(503, 'not_configured');
  const payload = decodeHandle(cfg.handleKey, handle);
  if (payload.x <= nowSecs) throw new RelayError(401, 'bad_handle', 'expired');
  if (payload.b !== cfg.topic) throw new RelayError(401, 'bad_handle', 'topic');
  if (cfg.revoked.has(payload.r)) throw new RelayError(401, 'bad_handle', 'revoked');
  return payload;
}

// Validates a Mac push request and builds the APNs request (without the `authorization` header,
// which the adapter adds from its cached JWT). The relay composes the alert itself.
function buildPush(body, cfg, nowSecs) {
  if (!cfg.apns) throw new RelayError(503, 'not_configured');
  if (!body || typeof body !== 'object') throw new RelayError(400, 'bad_request', 'body');
  const h = resolveHandle(body.handle, cfg, nowSecs);
  if (body.kind !== 'alert' && body.kind !== 'replace') throw new RelayError(400, 'bad_request', 'kind');
  if (!LOC_KEYS.has(body.locKey)) throw new RelayError(400, 'bad_request', 'locKey');
  if (typeof body.threadId !== 'string' || !THREAD_RE.test(body.threadId)) {
    throw new RelayError(400, 'bad_request', 'threadId');
  }
  const collapseId = body.collapseId;
  if (
    typeof collapseId !== 'string' ||
    collapseId.length === 0 ||
    Buffer.byteLength(collapseId) > 64 ||
    !/^[\x21-\x7e]+$/.test(collapseId)
  ) {
    throw new RelayError(400, 'bad_request', 'collapseId');
  }
  const ah = body.ah;
  if (
    !ah ||
    typeof ah !== 'object' ||
    typeof ah.f !== 'string' ||
    ah.f.length > 64 ||
    !Number.isInteger(ah.k) ||
    typeof ah.n !== 'string' ||
    ah.n.length > 32 ||
    typeof ah.c !== 'string'
  ) {
    throw new RelayError(400, 'bad_request', 'ah');
  }

  const aps = {
    alert: { 'loc-key': body.locKey },
    'mutable-content': 1,
    'thread-id': body.threadId,
  };
  if (body.kind === 'alert') {
    aps.sound = 'default';
  } else {
    aps['interruption-level'] = 'passive';
  }
  const payload = JSON.stringify({ aps, ah: { f: ah.f, k: ah.k, n: ah.n, c: ah.c } });
  if (Buffer.byteLength(payload) > MAX_PUSH_BYTES) throw new RelayError(413, 'too_large');

  const headers = {
    'apns-topic': cfg.apns.topic,
    'apns-push-type': 'alert',
    'apns-priority': body.kind === 'alert' ? '10' : '5',
    'apns-collapse-id': collapseId,
    'apns-expiration': String(nowSecs + (body.kind === 'alert' ? 3600 : 600)),
    'content-type': 'application/json',
  };
  return { rid: h.r, env: h.e, path: `/3/device/${h.dt}`, headers, body: payload };
}

// Maps an APNs response to the relay's HTTP answer.
function mapApnsResult(res) {
  if (res.status === 200) return { status: 200, body: { ok: true, apnsId: res.apnsId || null } };
  const gone =
    res.status === 410 ||
    res.reason === 'BadDeviceToken' ||
    res.reason === 'Unregistered' ||
    res.reason === 'DeviceTokenNotForTopic';
  if (gone) return { status: 410, body: { error: 'gone', reason: res.reason || null } };
  if (res.status === 413) return { status: 413, body: { error: 'too_large' } };
  return {
    status: 502,
    body: { error: 'apns_error', status: res.status, reason: res.reason || null },
  };
}

// ---------------------------------------------------------------- JWT

// ES256 provider token. `iat` is quantized to 30-minute windows and cached so that many cold-start
// instances do not trigger APNs's token-update rate limit (TooManyProviderTokenUpdates).
function makeJwtSigner(apns) {
  let cached = { iat: -1, token: '' };
  return function jwt(nowSecs) {
    const iat = Math.floor(nowSecs / JWT_WINDOW_SECS) * JWT_WINDOW_SECS;
    if (cached.iat === iat) return cached.token;
    const header = b64u(JSON.stringify({ alg: 'ES256', kid: apns.keyId }));
    const claims = b64u(JSON.stringify({ iss: apns.teamId, iat }));
    const sig = crypto.sign('sha256', Buffer.from(`${header}.${claims}`), {
      key: apns.p8,
      dsaEncoding: 'ieee-p1363',
    });
    cached = { iat, token: `${header}.${claims}.${b64u(sig)}` };
    return cached.token;
  };
}

// ---------------------------------------------------------------- rate limit

// Best-effort in-memory token bucket (not shared between instances).
class TokenBuckets {
  constructor({ capacity, refillPerSec, maxKeys = 2000 }) {
    this.capacity = capacity;
    this.refillPerSec = refillPerSec;
    this.maxKeys = maxKeys;
    this.map = new Map();
  }

  allow(key, nowMs) {
    let b = this.map.get(key);
    if (!b) {
      if (this.map.size >= this.maxKeys) this.map.delete(this.map.keys().next().value);
      b = { tokens: this.capacity, at: nowMs };
      this.map.set(key, b);
    }
    b.tokens = Math.min(this.capacity, b.tokens + ((nowMs - b.at) / 1000) * this.refillPerSec);
    b.at = nowMs;
    if (b.tokens < 1) return false;
    b.tokens -= 1;
    return true;
  }
}

module.exports = {
  HANDLE_LIFETIME_SECS,
  LOC_KEYS,
  MAX_PUSH_BYTES,
  RelayError,
  TokenBuckets,
  buildPush,
  decodeHandle,
  encodeHandle,
  makeJwtSigner,
  mapApnsResult,
  parseConfig,
  register,
};
