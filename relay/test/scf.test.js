'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const http = require('node:http');
const http2 = require('node:http2');
const { once } = require('node:events');
const { clientIpFromForwardedFor, createApnsSender, createServer } = require('../src/adapters/scf');

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

// What the SCF gateway sends: the real client first, then the gateway and the function host.
const viaGateway = (client) => `${client}, 11.163.3.223, 10.130.162.47`;

async function registerStatus(base, forwardedFor) {
  const res = await fetch(`${base}/v1/register`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', ...(forwardedFor ? { 'x-forwarded-for': forwardedFor } : {}) },
    body: JSON.stringify({ deviceToken: 'b'.repeat(64), env: 'dev', bundleId: TOPIC, appVer: '1' }),
  });
  await res.arrayBuffer();
  return res.status;
}

async function registerMany(base, forwardedFor, count) {
  const statuses = [];
  for (let i = 0; i < count; i++) statuses.push(await registerStatus(base, forwardedFor(i)));
  return statuses;
}

test('client address is the entry before the two trusted hops', () => {
  const ip = (header, hops = 2) => clientIpFromForwardedFor(header, hops);
  assert.equal(ip(viaGateway('198.51.100.7')), '198.51.100.7');
  assert.equal(ip('  198.51.100.7 ,11.1.1.1,  10.1.1.1 '), '198.51.100.7');
  assert.equal(ip(viaGateway('2001:db8::5')), '2001:db8::5');
  // Whatever the caller put in front of the gateway's entries is ignored.
  assert.equal(ip(`1.2.3.4, 5.6.7.8, ${viaGateway('198.51.100.7')}`), '198.51.100.7');
  // The hop count moves the pick; zero hops means the last entry itself.
  assert.equal(ip(viaGateway('198.51.100.7'), 1), '11.163.3.223');
  assert.equal(ip(viaGateway('198.51.100.7'), 0), '10.130.162.47');
});

test('client address is null when the header cannot be trusted', () => {
  const ip = (header, hops = 2) => clientIpFromForwardedFor(header, hops);
  for (const header of [undefined, null, 42, '', ' ', '198.51.100.7', '11.163.3.223, 10.130.162.47']) {
    assert.equal(ip(header), null, String(header));
  }
  for (const client of ['garbage', '', '198.51.100.7:4433', '[2001:db8::5]', 'unknown']) {
    assert.equal(ip(viaGateway(client)), null, client);
  }
});

test('registration is limited per client address, not for everybody at once', async () => {
  const r = await startRelay({ server: { now: () => 1_800_000_000_000 } });
  const burst = await registerMany(r.base, () => viaGateway('198.51.100.1'), 31);
  assert.deepEqual(burst.slice(0, 30), Array(30).fill(200));
  assert.equal(burst[30], 429);
  // Another client is not affected by the first one's exhausted bucket.
  assert.equal(await registerStatus(r.base, viaGateway('198.51.100.2')), 200);
  await r.close();
});

test('a forged X-Forwarded-For prefix does not buy a fresh bucket', async () => {
  const r = await startRelay({ server: { now: () => 1_800_000_000_000 } });
  const forged = (i) => `9.9.9.${i}, ${viaGateway('198.51.100.1')}`;
  const statuses = await registerMany(r.base, forged, 31);
  assert.deepEqual(statuses.slice(0, 30), Array(30).fill(200));
  assert.equal(statuses[30], 429);
  await r.close();
});

test('IPv6 clients share a bucket per /64', async () => {
  const r = await startRelay({ server: { now: () => 1_800_000_000_000 } });
  const sameNet = (i) => viaGateway(`2001:db8:1:2::${(i + 1).toString(16)}`);
  const statuses = await registerMany(r.base, sameNet, 31);
  assert.equal(statuses.filter((s) => s === 200).length, 30);
  assert.equal(statuses[30], 429);
  assert.equal(await registerStatus(r.base, viaGateway('2001:db8:1:3::1')), 200);
  await r.close();
});

test('requests without a usable client address share one small bucket', async () => {
  const r = await startRelay({ server: { now: () => 1_800_000_000_000 } });
  const mixed = (i) => (i % 2 ? 'bogus' : undefined);
  const statuses = await registerMany(r.base, mixed, 31);
  assert.equal(statuses.filter((s) => s === 200).length, 30);
  assert.equal(statuses[30], 429);
  // Identified clients still have their own buckets.
  assert.equal(await registerStatus(r.base, viaGateway('198.51.100.1')), 200);
  await r.close();
});

test('a global backstop caps registration across many clients', async () => {
  const r = await startRelay({ server: { now: () => 1_800_000_000_000 } });
  const distinct = (i) => viaGateway(`198.51.${Math.floor(i / 250)}.${(i % 250) + 1}`);
  const statuses = await registerMany(r.base, distinct, 201);
  assert.equal(statuses.filter((s) => s === 200).length, 200);
  assert.equal(statuses[200], 429);
  await r.close();
});

test('TRUSTED_PROXY_HOPS selects which forwarded address is the client', async () => {
  const env = { ...makeEnv(), TRUSTED_PROXY_HOPS: '1' };
  const r = await startRelay({ server: { env, now: () => 1_800_000_000_000 } });
  const oneHop = (client) => `${client}, 10.130.162.47`;
  const burst = await registerMany(r.base, () => oneHop('198.51.100.1'), 31);
  assert.equal(burst[30], 429);
  assert.equal(await registerStatus(r.base, oneHop('198.51.100.2')), 200);
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
