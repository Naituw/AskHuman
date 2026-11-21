'use strict'

// Self-tests of the compatibility suite, run offline against real (Aedes) brokers on loopback: two
// compliant ones that keep accounts apart in the two ways real brokers do, one with no separation at all,
// and one per way a broker can be unfit for the channel. Each check has to say the right thing both ways.

const { describe, it } = require('node:test')
const assert = require('node:assert/strict')
const net = require('node:net')
const path = require('node:path')
const { execFile } = require('node:child_process')
const { startBroker } = require('./fixtures.cjs')
const { runCompat, TRANSPORT_CHECKS, percentiles, describeError } = require('../lib/checks.cjs')
const { summarize, toMarkdown, toJson, textSummary, line } = require('../lib/report.cjs')
const { MqttClient } = require('../lib/client.cjs')
const { openPipe } = require('../lib/transport.cjs')

const FAST = { keepaliveSecs: 1, slowKeepaliveSecs: 1, settleMs: 400, willWaitMs: 3000 }
const KINDS = ['tcp', 'ws']

async function run(brokerOptions, overrides = {}) {
  const broker = await startBroker(brokerOptions)
  try {
    const { results } = await runCompat({
      host: '127.0.0.1',
      tlsPort: broker.port,
      wsPort: broker.wsPort,
      wsPath: '/mqtt',
      insecureLocal: true,
      kinds: KINDS,
      user: 'alice',
      pass: 'pw-alice',
      user2: 'bob',
      pass2: 'pw-bob',
      ...FAST,
      ...overrides,
    })
    return results
  } finally {
    await broker.stop()
  }
}

const find = (results, id, transport) =>
  results.find((r) => r.id === id && (transport === undefined || r.transport === transport))

function assertAllRequiredPass(results) {
  for (const kind of KINDS) {
    for (const id of TRANSPORT_CHECKS) {
      const result = find(results, id, kind)
      assert.equal(result.status, 'ok', `${kind} ${id}: ${result.note}`)
    }
  }
  assert.equal(summarize(results).pass, true)
}

const cli = (script, args, env) =>
  new Promise((resolve) => {
    execFile(
      process.execPath,
      [path.join(__dirname, '..', script), ...args],
      { env: { ...process.env, AH_PROBE_PASSWORD: '', AH_PROBE_PASSWORD2: '', ...env }, timeout: 120000 },
      (error, stdout, stderr) => resolve({ code: error ? error.code : 0, stdout, stderr }),
    )
  })

describe('compatibility suite against compliant brokers', { concurrency: 4 }, () => {
  it('passes every check on a broker that refuses out-of-namespace access', async () => {
    const results = await run({ acl: 'suback' })
    assertAllRequiredPass(results)
    assert.equal(find(results, 'isolation').status, 'ok')
    assert.equal(find(results, 'cross-user').status, 'ok')
    assert.match(find(results, 'isolation').note, /subscriptions refused/)
  })

  it('passes every check on a broker that accepts everything and drops at delivery', async () => {
    const results = await run({ acl: 'forward' })
    assertAllRequiredPass(results)
    assert.equal(find(results, 'isolation').status, 'ok')
    assert.match(find(results, 'isolation').note, /accepted, then dropped at delivery/)
    assert.equal(find(results, 'cross-user').status, 'ok')
  })

  it('warns, without failing, when accounts are not separated at all', async () => {
    const results = await run({ acl: 'none' })
    assertAllRequiredPass(results)
    const isolation = find(results, 'isolation')
    assert.equal(isolation.status, 'warn')
    assert.match(isolation.note, /another user/)
    assert.match(isolation.note, /outside ah\//)
    assert.equal(find(results, 'cross-user').status, 'warn')
  })

  it('warns, without failing, when a clean DISCONNECT still publishes the will', async () => {
    const results = await run({ acl: 'suback', quirks: { willOnClean: true } })
    for (const kind of KINDS) {
      const clean = find(results, 'will-clean', kind)
      assert.equal(clean.status, 'warn', kind)
      assert.equal(clean.level, 'should')
      assert.match(clean.note, /although the client sent DISCONNECT/)
      for (const id of TRANSPORT_CHECKS) {
        if (id !== 'will-clean') assert.equal(find(results, id, kind).status, 'ok', `${kind} ${id}: ${find(results, id, kind).note}`)
      }
    }
    const s = summarize(results)
    assert.equal(s.pass, true)
    assert.equal(s.warn, 2)
    assert.deepEqual(s.mustFailed, [])
    assert.match(textSummary(results), /->\s+COMPATIBLE \(with warnings\)$/)
    assert.match(textSummary(results), /Warnings \(the channel works/)
    assert.match(textSummary(results, { strict: true }), /->\s+NOT COMPATIBLE \(--strict counts warnings as failures\)$/)
  })

  it('leaves no retained message behind', async () => {
    const broker = await startBroker({ acl: 'suback' })
    try {
      await runCompat({
        host: '127.0.0.1', tlsPort: broker.port, wsPort: 0, wsPath: '/mqtt', insecureLocal: true,
        kinds: ['tcp'], user: 'alice', pass: 'pw-alice', user2: 'bob', pass2: 'pw-bob', ...FAST,
      })
      const client = new MqttClient({ kind: 'tcp', host: '127.0.0.1', port: broker.port, insecureLocal: true, timeoutMs: 5000 }, 'audit')
      await client.open()
      assert.equal(await client.login({ user: 'alice', pass: 'pw-alice' }), 0)
      await client.subscribe('ah/alice/#')
      assert.equal(await client.expectPublish(() => true, 600), null)
      client.close()
      const bob = new MqttClient({ kind: 'tcp', host: '127.0.0.1', port: broker.port, insecureLocal: true, timeoutMs: 5000 }, 'audit-bob')
      await bob.open()
      assert.equal(await bob.login({ user: 'bob', pass: 'pw-bob' }), 0)
      await bob.subscribe('ah/bob/#')
      assert.equal(await bob.expectPublish(() => true, 600), null)
      bob.close()
    } finally {
      await broker.stop()
    }
  })

  it('measures the silent-loss will and the WebSocket close when asked to', async () => {
    const results = await run({ acl: 'suback' }, { slow: true })
    assert.equal(find(results, 'silent-will').status, 'info')
    assert.match(find(results, 'silent-will').note, /the will arrived after \d/)
    assert.equal(find(results, 'ws-close-will').status, 'info')
    assert.equal(find(results, 'ws-close-will').transport, 'ws')
    assert.match(find(results, 'ws-close-will').note, /the will arrived/)
  })

  it('skips the idle check with quick', async () => {
    const results = await run({ acl: 'suback' }, { quick: true })
    assert.deepEqual(results.filter((r) => r.id === 'keepalive').map((r) => r.status), ['skip', 'skip'])
    assert.equal(summarize(results).pass, true)
  })

  it('runs only the isolation checks when asked to', async () => {
    const leaking = await run({ acl: 'none' }, { only: 'isolation' })
    assert.deepEqual(leaking.map((r) => r.id), ['connect', 'isolation', 'cross-user'])
    assert.equal(find(leaking, 'isolation').status, 'warn')
    const isolated = await run({ acl: 'forward' }, { only: 'isolation' })
    assert.deepEqual(isolated.map((r) => r.status), ['ok', 'ok', 'ok'])
  })
})

describe('compatibility suite against unfit brokers', { concurrency: 4 }, () => {
  const expectFail = async (quirks, ids, extra) => {
    const results = await run({ acl: 'suback', quirks })
    for (const kind of KINDS) {
      for (const id of ids) assert.equal(find(results, id, kind).status, 'fail', `${kind} ${id}`)
      for (const id of TRANSPORT_CHECKS) {
        if (!ids.includes(id)) assert.equal(find(results, id, kind).status, 'ok', `${kind} ${id}: ${find(results, id, kind).note}`)
      }
    }
    assert.equal(summarize(results).pass, false)
    if (extra) extra(results)
    return results
  }

  it('fails retained delivery and the retained will when the broker drops the retain flag', () =>
    expectFail({ noRetain: true }, ['retained', 'will'], (results) => {
      assert.match(find(results, 'retained', 'tcp').note, /received nothing/)
      assert.match(find(results, 'will', 'tcp').note, /not kept as the retained message/)
    }))

  it('fails when the will is never published', () =>
    expectFail({ noWill: true }, ['will'], (results) => assert.match(find(results, 'will', 'tcp').note, /no will within/)))

  it('fails when the will is published but not retained', () =>
    expectFail({ willNotRetained: true }, ['will'], (results) => assert.match(find(results, 'will', 'ws').note, /not kept as the retained message/)))

  it('fails when messages above a size limit are refused, and says how far it got', () =>
    expectFail({ maxPayload: 300000 }, ['big'], (results) => {
      assert.match(find(results, 'big', 'tcp').note, /332,000 B: the broker closed the connection; worked up to 100,000 B/)
    }))

  it('fails when the presence wildcard subscription is refused', () =>
    expectFail({ refuseWildcard: true }, ['subscribe'], (results) => assert.match(find(results, 'subscribe', 'tcp').note, /presence wildcard.*refused/)))

  it('fails when idle sessions are cut off', () =>
    expectFail({ maxSessionMs: 1200 }, ['keepalive'], (results) => assert.match(find(results, 'keepalive', 'tcp').note, /closed an idle session/)))
})

describe('compatibility suite when the broker cannot be used', { concurrency: 4 }, () => {
  it('reports a refused login without ever printing the password', async () => {
    const secret = 'wrong-secret-value-9f3'
    const results = await run({ acl: 'suback' }, { pass: secret })
    for (const kind of KINDS) {
      assert.equal(find(results, 'connect', kind).status, 'fail')
      assert.match(find(results, 'connect', kind).note, /bad user name or password/)
      for (const id of TRANSPORT_CHECKS.slice(1)) assert.equal(find(results, id, kind).status, 'skip')
    }
    assert.equal(find(results, 'isolation').status, 'skip')
    const everything = JSON.stringify(results) + results.map(line).join('\n') + textSummary(results)
    assert.equal(everything.includes(secret), false)
  })

  it('reports a closed port as refused', async () => {
    const probe = net.createServer()
    await new Promise((resolve) => probe.listen(0, '127.0.0.1', resolve))
    const { port } = probe.address()
    await new Promise((resolve) => probe.close(resolve))
    const { results } = await runCompat({
      host: '127.0.0.1', tlsPort: port, wsPort: 0, wsPath: '/mqtt', insecureLocal: true, kinds: ['tcp'],
      user: 'alice', pass: 'pw-alice', ...FAST,
    })
    assert.equal(find(results, 'connect', 'tcp').status, 'fail')
    assert.match(find(results, 'connect', 'tcp').note, /connection refused/)
    assert.equal(find(results, 'subscribe', 'tcp').status, 'skip')
  })

  it('gives up on a server that accepts the connection but never answers the WebSocket upgrade', async () => {
    const silent = net.createServer((socket) => socket.on('data', () => {}).on('error', () => {}))
    await new Promise((resolve) => silent.listen(0, '127.0.0.1', resolve))
    try {
      const { results } = await runCompat({
        host: '127.0.0.1', tlsPort: 1, wsPort: silent.address().port, wsPath: '/mqtt', insecureLocal: true,
        kinds: ['ws'], user: 'alice', pass: 'pw-alice', ...FAST, timeoutMs: 500,
      })
      assert.equal(find(results, 'connect', 'ws').status, 'fail')
      assert.match(find(results, 'connect', 'ws').note, /no answer to the WebSocket upgrade within 500 ms/)
    } finally {
      silent.close()
    }
  })

  it('names the cause of common network and TLS errors', () => {
    assert.match(describeError({ code: 'ENOTFOUND' }), /does not resolve/)
    assert.match(describeError({ code: 'ECONNREFUSED' }), /connection refused/)
    assert.match(describeError({ code: 'CERT_HAS_EXPIRED' }), /not publicly trusted \(CERT_HAS_EXPIRED\)/)
    assert.match(describeError({ code: 'DEPTH_ZERO_SELF_SIGNED_CERT' }), /not publicly trusted/)
    assert.match(describeError(new Error('no connection within 10000 ms')), /time limit/)
    assert.equal(describeError(new Error('something else')), 'something else')
  })

  it('refuses plain transports unless they target loopback explicitly', async () => {
    await assert.rejects(openPipe({ kind: 'tcp', host: '127.0.0.1', port: 1 }), /only work with --insecure-local/)
    await assert.rejects(openPipe({ kind: 'ws', host: 'example.com', port: 1, insecureLocal: true }), /only work with --insecure-local/)
  })
})

describe('report', () => {
  const results = [
    { id: 'connect', level: 'must', title: 'connect', transport: 'tls', status: 'ok', note: 'open 10 ms' },
    { id: 'connect', level: 'must', title: 'connect', transport: 'wss', status: 'fail', note: 'refused | quoted' },
    { id: 'isolation', level: 'should', title: 'isolation', transport: 'tls', status: 'warn', note: 'leaks' },
  ]

  it('summarises counts and the verdict', () => {
    const s = summarize(results)
    assert.deepEqual([s.ok, s.fail, s.warn, s.pass], [1, 1, 1, false])
    assert.deepEqual(s.mustFailed, ['connect'])
    assert.match(textSummary(results), /NOT COMPATIBLE/)
    assert.match(textSummary(results.slice(0, 1)), /COMPATIBLE/)
    assert.match(textSummary(results.slice(2), { only: 'isolation', strict: true }), /isolation checks did not pass/)
  })

  it('renders a Markdown table with a column per transport and escapes pipes', () => {
    const markdown = toMarkdown({ label: 'Example', date: '2026-01-01', node: '24.0.0' }, results)
    assert.match(markdown, /\*\*Compatibility run: Example \(2026-01-01, node 24\.0\.0\)\*\*/)
    assert.match(markdown, /\| Check \| Level \| tls \| wss \|/)
    assert.match(markdown, /❌ refused \\\| quoted/)
  })

  it('renders JSON with a summary', () => {
    const parsed = JSON.parse(toJson({ label: 'x' }, results))
    assert.equal(parsed.summary.fail, 1)
    assert.equal(parsed.results.length, 3)
  })

  it('computes percentiles', () => {
    assert.deepEqual(percentiles([5, 1, 3, 2, 4]), { p50: 3, p95: 5 })
    assert.deepEqual(percentiles([7]), { p50: 7, p95: 7 })
  })
})

describe('command line', { concurrency: 4 }, () => {
  const endpoint = (broker) => ['--host', '127.0.0.1', '--port', String(broker.port), '--insecure-local', '--user', 'alice', '--keepalive-secs', '1']

  it('exits 0 for a compatible broker, prints reports on stdout and never the password', async () => {
    const broker = await startBroker({ acl: 'suback' })
    try {
      const out = await cli('compat.cjs', [...endpoint(broker), '--json', '--markdown'], { AH_PROBE_PASSWORD: 'pw-alice' })
      assert.equal(out.code, 0, out.stderr)
      assert.match(out.stderr, /COMPATIBLE/)
      const json = out.stdout.slice(0, out.stdout.indexOf('\n**Compatibility run'))
      assert.equal(JSON.parse(json).summary.pass, true)
      assert.match(out.stdout, /\| Check \| Level \| tcp \|/)
      assert.equal((out.stdout + out.stderr).includes('pw-alice'), false)
    } finally {
      await broker.stop()
    }
  })

  it('exits 0 with a warning for a will after DISCONNECT, and 1 with --strict', async () => {
    const broker = await startBroker({ acl: 'suback', quirks: { willOnClean: true } })
    try {
      const plain = await cli('compat.cjs', [...endpoint(broker), '--quick'], { AH_PROBE_PASSWORD: 'pw-alice' })
      assert.equal(plain.code, 0, plain.stderr)
      assert.match(plain.stderr, /warn {2}tcp {2}will-clean/)
      assert.match(plain.stderr, /->\s+COMPATIBLE \(with warnings\)/)
      const strict = await cli('compat.cjs', [...endpoint(broker), '--quick', '--strict'], { AH_PROBE_PASSWORD: 'pw-alice' })
      assert.equal(strict.code, 1)
      assert.match(strict.stderr, /->\s+NOT COMPATIBLE \(--strict/)
    } finally {
      await broker.stop()
    }
  })

  it('exits 1 for a refused login, and 2 for a missing password or unknown argument', async () => {
    const broker = await startBroker({ acl: 'suback' })
    try {
      const refused = await cli('compat.cjs', endpoint(broker), { AH_PROBE_PASSWORD: 'pw-nope' })
      assert.equal(refused.code, 1)
      assert.match(refused.stderr, /bad user name or password/)
      assert.equal((refused.stdout + refused.stderr).includes('pw-nope'), false)
      const missing = await cli('compat.cjs', endpoint(broker), {})
      assert.equal(missing.code, 2)
      assert.match(missing.stderr, /AH_PROBE_PASSWORD/)
      const unknown = await cli('compat.cjs', ['--nope'], { AH_PROBE_PASSWORD: 'x' })
      assert.equal(unknown.code, 2)
    } finally {
      await broker.stop()
    }
  })

  it('keeps acl-check.cjs working: exit 0 when accounts are separated, 1 on a leak', async () => {
    const isolated = await startBroker({ acl: 'forward' })
    const leaking = await startBroker({ acl: 'none' })
    try {
      const args = (broker) => [...endpoint(broker), '--other', 'someone', '--protocol', '5']
      const good = await cli('acl-check.cjs', args(isolated), { AH_PROBE_PASSWORD: 'pw-alice' })
      assert.equal(good.code, 0, good.stderr)
      assert.match(good.stderr, /--other is no longer needed/)
      assert.match(good.stderr, /isolation checks passed/)
      const bad = await cli('acl-check.cjs', args(leaking), { AH_PROBE_PASSWORD: 'pw-alice' })
      assert.equal(bad.code, 1)
      assert.match(bad.stderr, /outside ah\//)
    } finally {
      await isolated.stop()
      await leaking.stop()
    }
  })
})
