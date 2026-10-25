'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const http = require('node:http');
const http2 = require('node:http2');
const { once } = require('node:events');
const { createApnsSender, createServer } = require('../src/adapters/scf');

const TOPIC = 'com.naituw.humaninloop.ios';

function makeEnv() {
  const { privateKey } = crypto.generateKeyPairSync('ec', { namedCurve: 'P-256' });
  return {
    APNS_KEY_P8: privateKey.export({ type: 'pkcs8', format: 'pem' }),
    APNS_KEY_ID: 'ABC123DEFG',
    APNS_TEAM_ID: 'TEAM123456',
    APNS_TOPIC: TOPIC,
    RELAY_HANDLE_KEY: crypto.randomBytes(32).toString('base64'),
  };
}

async function startRelay(opts = {}) {
  const sent = [];
  const sendApns =
    opts.sendApns ||
    (async (args) => {
      sent.push(args);
      return { status: 200, reason: null, apnsId: 'apns-1' };
    });
  const server = createServer({ env: makeEnv(), sendApns, retryDelayMs: 1, log: () => {}, ...opts.server });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const base = `http://127.0.0.1:${server.address().port}`;
  return { server, base, sent, close: () => new Promise((r) => server.close(r)) };
}

async function call(base, method, path, body) {
  const res = await fetch(base + path, {
    method,
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : typeof body === 'string' ? body : JSON.stringify(body),
  });
  return { status: res.status, json: await res.json() };
}

async function registerHandle(base) {
  const r = await call(base, 'POST', '/v1/register', {
    deviceToken: 'b'.repeat(64),
    env: 'dev',
    bundleId: TOPIC,
    appVer: '1',
  });
  assert.equal(r.status, 200);
  return r.json.handle;
}

const pushBody = (handle, extra = {}) => ({
  handle,
  kind: 'alert',
  locKey: 'PUSH_ASK',
  threadId: 'mac-1',
  collapseId: 'req-1',
  ah: { f: 'm', k: 1, n: 'n', c: 'c' },
  ...extra,
});

test('health reports configuration without secrets', async () => {
  const r = await startRelay();
  const res = await call(r.base, 'GET', '/v1/health');
  assert.deepEqual(res.json, { ok: true, v: 1, apns: true });
  await r.close();
});

test('health works behind a path prefix', async () => {
  const r = await startRelay();
  const res = await call(r.base, 'GET', '/release/askhuman/v1/health');
  assert.equal(res.status, 200);
  await r.close();
});

test('register then push reaches APNs with bearer auth and the dev environment', async () => {
  const r = await startRelay();
  const handle = await registerHandle(r.base);
  const res = await call(r.base, 'POST', '/v1/push', pushBody(handle));
  assert.equal(res.status, 200);
  assert.deepEqual(res.json, { ok: true, apnsId: 'apns-1' });
  assert.equal(r.sent.length, 1);
  assert.equal(r.sent[0].env, 'dev');
  assert.equal(r.sent[0].path, `/3/device/${'b'.repeat(64)}`);
  assert.match(r.sent[0].headers.authorization, /^bearer [\w-]+\.[\w-]+\.[\w-]+$/);
  await r.close();
});

test('APNs "gone" answers become 410', async () => {
  const r = await startRelay({ sendApns: async () => ({ status: 410, reason: 'Unregistered' }) });
  const handle = await registerHandle(r.base);
  const res = await call(r.base, 'POST', '/v1/push', pushBody(handle));
  assert.equal(res.status, 410);
  assert.equal(res.json.error, 'gone');
  await r.close();
});

test('TooManyProviderTokenUpdates is retried once', async () => {
  let calls = 0;
  const r = await startRelay({
    sendApns: async () => {
      calls += 1;
      return calls === 1
        ? { status: 429, reason: 'TooManyProviderTokenUpdates' }
        : { status: 200, apnsId: 'ok' };
    },
  });
  const handle = await registerHandle(r.base);
  const res = await call(r.base, 'POST', '/v1/push', pushBody(handle));
  assert.equal(res.status, 200);
  assert.equal(calls, 2);
  await r.close();
});

test('network failures to APNs become 502', async () => {
  const r = await startRelay({
    sendApns: async () => {
      throw new Error('boom');
    },
  });
  const handle = await registerHandle(r.base);
  const res = await call(r.base, 'POST', '/v1/push', pushBody(handle));
  assert.equal(res.status, 502);
  assert.equal(res.json.error, 'apns_error');
  await r.close();
});

test('bad handle, bad JSON, unknown route and oversize body', async () => {
  const r = await startRelay();
  assert.equal((await call(r.base, 'POST', '/v1/push', pushBody('v1.nope'))).status, 401);
  assert.equal((await call(r.base, 'POST', '/v1/push', '{not json')).status, 400);
  assert.equal((await call(r.base, 'GET', '/v2/whatever')).status, 404);
  assert.equal((await call(r.base, 'POST', '/v1/register', 'x'.repeat(20_000))).status, 413);
  await r.close();
});

test('register refuses another bundle id', async () => {
  const r = await startRelay();
  const res = await call(r.base, 'POST', '/v1/register', {
    deviceToken: 'c'.repeat(64),
    env: 'prod',
    bundleId: 'com.other',
  });
  assert.equal(res.status, 400);
  await r.close();
});

test('per-handle rate limit returns 429 after the burst', async () => {
  const r = await startRelay({ server: { now: () => 1_800_000_000_000 } });
  const handle = await registerHandle(r.base);
  let last = 0;
  for (let i = 0; i < 12; i++) {
    last = (await call(r.base, 'POST', '/v1/push', pushBody(handle))).status;
  }
  assert.equal(last, 429);
  await r.close();
});

test('unconfigured relay answers 503 and a health check says apns=false', async () => {
  const server = createServer({ env: {}, log: () => {} });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const base = `http://127.0.0.1:${server.address().port}`;
  assert.deepEqual((await call(base, 'GET', '/v1/health')).json, { ok: true, v: 1, apns: false });
  const res = await call(base, 'POST', '/v1/push', pushBody('v1.x'));
  assert.equal(res.status, 503);
  await new Promise((r) => server.close(r));
});

test('the HTTP/2 sender posts to /3/device and parses APNs reasons', async () => {
  const seen = [];
  const apns = http2.createServer();
  apns.on('stream', (stream, headers) => {
    const chunks = [];
    stream.on('data', (c) => chunks.push(c));
    stream.on('end', () => {
      seen.push({ headers, body: Buffer.concat(chunks).toString() });
      if (seen.length === 1) {
        stream.respond({ ':status': 200, 'apns-id': 'abc' });
        stream.end();
      } else {
        stream.respond({ ':status': 400, 'content-type': 'application/json' });
        stream.end(JSON.stringify({ reason: 'BadDeviceToken' }));
      }
    });
  });
  apns.listen(0, '127.0.0.1');
  await once(apns, 'listening');
  const origin = `http://127.0.0.1:${apns.address().port}`;
  const send = createApnsSender({ origins: { prod: origin, dev: origin }, timeoutMs: 2000 });
  const args = {
    env: 'prod',
    path: '/3/device/dead',
    headers: { 'apns-topic': TOPIC, authorization: 'bearer t' },
    body: '{"aps":{}}',
  };
  assert.deepEqual(await send(args), { status: 200, reason: null, apnsId: 'abc' });
  assert.deepEqual(await send(args), { status: 400, reason: 'BadDeviceToken', apnsId: null });
  assert.equal(seen[0].headers[':path'], '/3/device/dead');
  assert.equal(seen[0].headers['apns-topic'], TOPIC);
  assert.equal(seen[0].body, '{"aps":{}}');
  // The sender keeps a pooled session; close it so it does not hold the test process open.
  send.close();
  apns.close();
});

test('server module is a plain http server', () => {
  assert.ok(createServer({ env: {}, log: () => {} }) instanceof http.Server);
});
