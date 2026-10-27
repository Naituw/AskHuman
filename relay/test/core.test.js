'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const core = require('../src/core');

const TOPIC = 'com.naituw.humaninloop.ios';
const NOW = 1_800_000_000;

function makeEnv(overrides = {}) {
  const { privateKey } = crypto.generateKeyPairSync('ec', { namedCurve: 'P-256' });
  const p8 = privateKey.export({ type: 'pkcs8', format: 'pem' });
  return {
    APNS_KEY_P8: p8,
    APNS_KEY_ID: 'ABC123DEFG',
    APNS_TEAM_ID: 'TEAM123456',
    APNS_TOPIC: TOPIC,
    RELAY_HANDLE_KEY: crypto.randomBytes(32).toString('base64'),
    ...overrides,
  };
}

const TOKEN = 'a'.repeat(64);

function registerOk(cfg, nowSecs = NOW) {
  return core.register({ deviceToken: TOKEN, env: 'prod', bundleId: TOPIC, appVer: '1.0' }, cfg, nowSecs);
}

function pushBody(handle, extra = {}) {
  return {
    handle,
    kind: 'alert',
    locKey: 'PUSH_ASK',
    threadId: 'mac-abc',
    collapseId: 'req-1',
    ah: { f: 'macid', k: 1, n: 'bm9uY2U', c: 'Y2lwaGVy' },
    ...extra,
  };
}

test('parseConfig rejects a handle key that is not 32 bytes', () => {
  assert.throws(() => core.parseConfig({ RELAY_HANDLE_KEY: Buffer.alloc(16).toString('base64') }));
});

test('parseConfig turns literal \\n into newlines in the p8 key', () => {
  const cfg = core.parseConfig(makeEnv({ APNS_KEY_P8: 'line1\\nline2' }));
  assert.equal(cfg.apns.p8, 'line1\nline2');
});

test('handle round trip carries device token, env and expiry', () => {
  const cfg = core.parseConfig(makeEnv());
  const { handle, expiresAt } = registerOk(cfg);
  assert.ok(handle.startsWith('v1.'));
  assert.equal(expiresAt, NOW + core.HANDLE_LIFETIME_SECS);
  const p = core.decodeHandle(cfg.handleKey, handle);
  assert.equal(p.dt, TOKEN);
  assert.equal(p.e, 'prod');
  assert.equal(p.b, TOPIC);
  assert.equal(p.x, expiresAt);
});

test('two registrations of the same token produce different handles', () => {
  const cfg = core.parseConfig(makeEnv());
  assert.notEqual(registerOk(cfg).handle, registerOk(cfg).handle);
});

test('register validates token, env and bundle id', () => {
  const cfg = core.parseConfig(makeEnv());
  const bad = (body) =>
    assert.throws(
      () => core.register(body, cfg, NOW),
      (e) => e instanceof core.RelayError && e.status === 400,
    );
  bad({ deviceToken: 'zz', env: 'prod', bundleId: TOPIC });
  bad({ deviceToken: 'ab', env: 'prod', bundleId: TOPIC });
  bad({ deviceToken: TOKEN, env: 'staging', bundleId: TOPIC });
  bad({ deviceToken: TOKEN, env: 'prod', bundleId: 'com.other.app' });
  bad(null);
});

test('register normalizes device token formatting', () => {
  const cfg = core.parseConfig(makeEnv());
  const { handle } = core.register(
    { deviceToken: `<${'AB'.repeat(32)}>`, env: 'dev', bundleId: TOPIC },
    cfg,
    NOW,
  );
  assert.equal(core.decodeHandle(cfg.handleKey, handle).dt, 'ab'.repeat(32));
});

test('register without configuration reports not_configured', () => {
  const cfg = core.parseConfig({});
  assert.throws(
    () => core.register({ deviceToken: TOKEN, env: 'prod', bundleId: TOPIC }, cfg, NOW),
    (e) => e.status === 503,
  );
});

test('decodeHandle rejects tampering, wrong key and garbage', () => {
  const cfg = core.parseConfig(makeEnv());
  const { handle } = registerOk(cfg);
  const flipped = handle.slice(0, -2) + (handle.endsWith('AA') ? 'BB' : 'AA');
  for (const h of [flipped, 'v1.', 'v2.abc', 'nope', '', null, 'v1.' + 'A'.repeat(20)]) {
    assert.throws(
      () => core.decodeHandle(cfg.handleKey, h),
      (e) => e.status === 401,
    );
  }
  assert.throws(
    () => core.decodeHandle(crypto.randomBytes(32), handle),
    (e) => e.status === 401,
  );
});

test('buildPush produces an APNs request with a relay-composed alert', () => {
  const cfg = core.parseConfig(makeEnv());
  const { handle } = registerOk(cfg);
  const req = core.buildPush(pushBody(handle), cfg, NOW);
  assert.equal(req.env, 'prod');
  assert.equal(req.path, `/3/device/${TOKEN}`);
  assert.equal(req.headers['apns-topic'], TOPIC);
  assert.equal(req.headers['apns-push-type'], 'alert');
  assert.equal(req.headers['apns-priority'], '10');
  assert.equal(req.headers['apns-collapse-id'], 'req-1');
  assert.equal(req.headers['apns-expiration'], String(NOW + 3600));
  const body = JSON.parse(req.body);
  assert.deepEqual(body.aps.alert, { 'loc-key': 'PUSH_ASK' });
  assert.equal(body.aps['mutable-content'], 1);
  assert.equal(body.aps['thread-id'], 'mac-abc');
  assert.equal(body.aps.sound, 'default');
  assert.deepEqual(body.ah, { f: 'macid', k: 1, n: 'bm9uY2U', c: 'Y2lwaGVy' });
});

test('replace pushes are passive, silent and low priority', () => {
  const cfg = core.parseConfig(makeEnv());
  const { handle } = registerOk(cfg);
  const req = core.buildPush(pushBody(handle, { kind: 'replace', locKey: 'PUSH_RESOLVED' }), cfg, NOW);
  const body = JSON.parse(req.body);
  assert.equal(body.aps['interruption-level'], 'passive');
  assert.equal(body.aps.sound, undefined);
  assert.equal(req.headers['apns-priority'], '5');
  assert.equal(req.headers['apns-expiration'], String(NOW + 600));
});

test('buildPush drops unknown fields so a Mac cannot inject alert text', () => {
  const cfg = core.parseConfig(makeEnv());
  const { handle } = registerOk(cfg);
  const body = pushBody(handle, { alert: 'pwned', aps: { alert: 'pwned' } });
  body.ah.extra = 'x'.repeat(10);
  const req = core.buildPush(body, cfg, NOW);
  const parsed = JSON.parse(req.body);
  assert.deepEqual(Object.keys(parsed).sort(), ['ah', 'aps']);
  assert.equal(parsed.aps.alert['loc-key'], 'PUSH_ASK');
  assert.equal(parsed.ah.extra, undefined);
});

test('buildPush validation errors', () => {
  const cfg = core.parseConfig(makeEnv());
  const { handle } = registerOk(cfg);
  const bad = (extra, status = 400) =>
    assert.throws(
      () => core.buildPush(pushBody(handle, extra), cfg, NOW),
      (e) => e instanceof core.RelayError && e.status === status,
    );
  bad({ locKey: 'PUSH_EVIL' });
  bad({ kind: 'silent' });
  bad({ threadId: 'bad thread!' });
  bad({ threadId: 'x'.repeat(65) });
  bad({ collapseId: '' });
  bad({ collapseId: 'x'.repeat(65) });
  bad({ collapseId: 'has space' });
  bad({ ah: { f: 'x', k: 'one', n: 'n', c: 'c' } });
  bad({ ah: null });
  bad({ ah: { f: 'x', k: 1, n: 'n', c: 'c'.repeat(4000) } }, 413);
});

test('buildPush rejects expired, revoked and foreign-topic handles', () => {
  const env = makeEnv();
  const cfg = core.parseConfig(env);
  const { handle, expiresAt } = registerOk(cfg);
  assert.throws(
    () => core.buildPush(pushBody(handle), cfg, expiresAt + 1),
    (e) => e.status === 401,
  );

  const rid = core.decodeHandle(cfg.handleKey, handle).r;
  const revokedCfg = core.parseConfig({ ...env, REVOKED_RIDS: `other, ${rid}` });
  assert.throws(
    () => core.buildPush(pushBody(handle), revokedCfg, NOW),
    (e) => e.status === 401,
  );

  const otherTopic = core.parseConfig({ ...env, APNS_TOPIC: 'com.other.app' });
  assert.throws(
    () => core.buildPush(pushBody(handle), otherTopic, NOW),
    (e) => e.status === 401,
  );
});

test('buildPush without APNs configuration reports not_configured', () => {
  const env = makeEnv();
  const cfg = core.parseConfig(env);
  const { handle } = registerOk(cfg);
  const noApns = core.parseConfig({ ...env, APNS_KEY_P8: '' });
  assert.throws(
    () => core.buildPush(pushBody(handle), noApns, NOW),
    (e) => e.status === 503,
  );
});

test('rotating the handle key invalidates existing handles', () => {
  const env = makeEnv();
  const { handle } = registerOk(core.parseConfig(env));
  const rotated = core.parseConfig({ ...env, RELAY_HANDLE_KEY: crypto.randomBytes(32).toString('base64') });
  assert.throws(
    () => core.buildPush(pushBody(handle), rotated, NOW),
    (e) => e.status === 401,
  );
});

test('mapApnsResult', () => {
  assert.deepEqual(core.mapApnsResult({ status: 200, apnsId: 'id1' }), {
    status: 200,
    body: { ok: true, apnsId: 'id1' },
  });
  for (const reason of ['BadDeviceToken', 'Unregistered', 'DeviceTokenNotForTopic']) {
    assert.equal(core.mapApnsResult({ status: 400, reason }).status, 410);
  }
  assert.equal(core.mapApnsResult({ status: 410, reason: 'Unregistered' }).status, 410);
  assert.equal(core.mapApnsResult({ status: 413 }).status, 413);
  const other = core.mapApnsResult({ status: 403, reason: 'InvalidProviderToken' });
  assert.equal(other.status, 502);
  assert.deepEqual(other.body, { error: 'apns_error', status: 403, reason: 'InvalidProviderToken' });
});

test('JWT is ES256 with the key id, team id and a quantized iat', () => {
  const cfg = core.parseConfig(makeEnv());
  const sign = core.makeJwtSigner(cfg.apns);
  const now = 1_800_000_123;
  const token = sign(now);
  const [h, c, s] = token.split('.');
  assert.deepEqual(JSON.parse(Buffer.from(h, 'base64url')), { alg: 'ES256', kid: 'ABC123DEFG' });
  const claims = JSON.parse(Buffer.from(c, 'base64url'));
  assert.equal(claims.iss, 'TEAM123456');
  assert.equal(claims.iat, Math.floor(now / 1800) * 1800);
  const pub = crypto.createPublicKey(cfg.apns.p8);
  assert.ok(
    crypto.verify('sha256', Buffer.from(`${h}.${c}`), { key: pub, dsaEncoding: 'ieee-p1363' }, Buffer.from(s, 'base64url')),
  );
});

test('JWT is cached within a window and renewed in the next one', () => {
  const cfg = core.parseConfig(makeEnv());
  const sign = core.makeJwtSigner(cfg.apns);
  const base = Math.floor(1_800_000_000 / 1800) * 1800;
  const a = sign(base + 10);
  assert.equal(sign(base + 1700), a);
  assert.notEqual(sign(base + 1800), a);
});

test('token buckets refill over time and evict old keys', () => {
  const b = new core.TokenBuckets({ capacity: 2, refillPerSec: 1, maxKeys: 2 });
  assert.equal(b.allow('a', 0), true);
  assert.equal(b.allow('a', 0), true);
  assert.equal(b.allow('a', 0), false);
  assert.equal(b.allow('a', 1000), true);
  b.allow('b', 0);
  b.allow('c', 0); // evicts the oldest key ('a')
  assert.equal(b.map.size, 2);
});
