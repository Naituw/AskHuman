'use strict'

// The checks of the compatibility suite. Each one answers a single question the AskHuman iOS channel
// depends on (see docs/wiki/ios-broker.md for the requirement list these come from).
//
// Levels:  must  — a failure means the broker cannot carry the channel
//          should — a failure is a warning (the channel works, a safeguard is missing or a harmless
//                   difference exists); it is reported as "warn", never as "fail"
//          info  — a measurement, never a verdict
//
// Every topic lives under ah/<username>/t/<run id>/ (the subtree the app's own connection test uses),
// retained topics are cleared at the end, and nothing is subscribed to outside ah/<username>/.

const crypto = require('node:crypto')
const { MqttClient, elapsedMs } = require('./client.cjs')

const CHECKS = {
  connect: { level: 'must', title: 'connect and log in (MQTT 3.1.1, clean session)' },
  subscribe: { level: 'must', title: 'subscribe with QoS 1 to the app\'s topic filters' },
  roundtrip: { level: 'must', title: '15 QoS 1 messages reach a second session of the same user' },
  retained: { level: 'must', title: 'a retained message reaches a later subscriber' },
  will: { level: 'must', title: 'a retained QoS 1 will is published when the connection drops' },
  big: { level: 'must', title: 'QoS 1 messages up to 900,000 bytes arrive intact' },
  keepalive: { level: 'must', title: 'the session survives idle time with PINGREQ' },
  // A will after DISCONNECT only adds a duplicate "offline": both apps publish their own, retained, before they
  // disconnect on purpose. Measured on EMQX over WSS (see docs/wiki/ios-broker.md), hence "should", not "must".
  'will-clean': { level: 'should', title: 'no will after a clean DISCONNECT' },
  isolation: { level: 'should', title: 'a user is only served inside ah/<own username>/ (checked at delivery)' },
  'cross-user': { level: 'should', title: 'a second credential cannot read or write the first one\'s topics' },
  'silent-will': { level: 'info', title: 'will after a silent loss (no packets, no close)' },
  'ws-close-will': { level: 'info', title: 'will after a WebSocket close with no MQTT DISCONNECT' },
}

/** Order of the per-transport checks (everything after `connect` needs a working login). */
const TRANSPORT_CHECKS = ['connect', 'subscribe', 'roundtrip', 'retained', 'will', 'will-clean', 'big', 'keepalive']

const BIG_SIZES = [100_000, 332_000, 900_000]

const CONNACK_REASONS = {
  1: 'unacceptable protocol version',
  2: 'client identifier rejected',
  3: 'server unavailable',
  4: 'bad user name or password',
  5: 'not authorized',
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const fmt = (ms) => `${Math.round(ms)} ms`

function percentiles(values) {
  const sorted = [...values].sort((a, b) => a - b)
  const at = (p) => sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(p * sorted.length) - 1))]
  return { p50: at(0.5), p95: at(0.95) }
}

const TLS_TRUST_CODES = new Set([
  'CERT_HAS_EXPIRED',
  'UNABLE_TO_VERIFY_LEAF_SIGNATURE',
  'UNABLE_TO_GET_ISSUER_CERT_LOCALLY',
  'SELF_SIGNED_CERT_IN_CHAIN',
  'DEPTH_ZERO_SELF_SIGNED_CERT',
  'ERR_TLS_CERT_ALTNAME_INVALID',
])

/** One readable sentence for a network or TLS error. Never includes anything the caller passed as a credential. */
function describeError(error) {
  const code = error && error.code
  if (code === 'ENOTFOUND') return 'the host name does not resolve'
  if (code === 'ECONNREFUSED') return 'connection refused (wrong port, or nothing listens there)'
  if (code === 'ECONNRESET') return 'the connection was reset'
  if (code === 'ETIMEDOUT' || /no connection within/.test(error && error.message)) {
    return 'no connection within the time limit (unreachable network, or a firewall)'
  }
  if (TLS_TRUST_CODES.has(code)) {
    return `the TLS certificate is not publicly trusted (${code}); the apps need a certificate that ordinary clients accept`
  }
  return (error && error.message) || String(error)
}

function loginError(code) {
  const error = new Error(
    code === null
      ? 'the broker closed the connection or did not answer the login'
      : `the broker refused the login: ${CONNACK_REASONS[code] || `return code ${code}`}`,
  )
  error.expected = true
  return error
}

function grantedText(granted) {
  if (granted === null) return 'no SUBACK'
  if (granted === 128) return 'refused'
  if (granted === 0) return 'downgraded to QoS 0'
  return `granted QoS ${granted}`
}

class Context {
  constructor(options) {
    this.options = {
      timeoutMs: 10000,
      keepaliveSecs: 10,
      slowKeepaliveSecs: 30,
      willWaitMs: 15000,
      // How long a message that must NOT arrive is waited for (a broker answers within a few round trips).
      settleMs: 2000,
      ...options,
    }
    this.run = this.options.run || crypto.randomBytes(3).toString('hex')
    this.user = this.options.user
    this.auth = { user: this.options.user, pass: this.options.pass }
    this.results = []
  }

  topic(rest) { return `ah/${this.user}/${rest}` }

  endpoint(kind) {
    const { host, tlsPort, wsPort, wsPath, insecureLocal, timeoutMs } = this.options
    const secure = kind === 'tls' || kind === 'wss'
    const isWs = kind === 'wss' || kind === 'ws'
    return { kind, host, port: isWs ? wsPort : tlsPort, path: wsPath, insecureLocal: insecureLocal && !secure, timeoutMs }
  }

  client(kind, label) { return new MqttClient(this.endpoint(kind), label) }

  emit(id, status, note, transport = null) {
    const { level, title } = CHECKS[id]
    // Only a "must" check can fail the run; the same problem in a "should" check is a warning.
    const reported = status === 'fail' && level !== 'must' ? 'warn' : status
    const result = { id, level, title, transport, status: reported, note: note || '' }
    this.results.push(result)
    if (this.options.onResult) this.options.onResult(result)
    return result
  }
}

/** Opens a session, logs in, and throws a readable error if either fails. */
async function connect(ctx, kind, label, extra = {}) {
  const client = ctx.client(kind, label)
  await client.open()
  const code = await client.login({ ...ctx.auth, ...extra })
  if (code !== 0) {
    client.kill()
    throw loginError(code)
  }
  return client
}

// ---------------------------------------------------------------------------- per-transport checks

async function checkKeepalive(ctx, kind, open) {
  const seconds = ctx.options.keepaliveSecs
  const idle = await open('idle', { keepalive: seconds, autoPing: false })
  const topic = ctx.topic(`t/${ctx.run}/ka`)
  for (let round = 1; round <= 2; round += 1) {
    await sleep(seconds * 1000)
    if (idle.closed) return ['fail', `the broker closed an idle session after about ${(round - 1) * seconds}-${round * seconds} s (keepalive ${seconds} s)`]
    if (!(await idle.ping())) return ['fail', `no PINGRESP after ${round * seconds} s idle (keepalive ${seconds} s)`]
  }
  const granted = await idle.subscribe(topic)
  const sent = granted !== null && granted !== 128 && (await idle.publish(topic, 'still here')) !== null
  const echoed = sent && Boolean(await idle.expectPublish((p) => p.topic === topic, 5000))
  if (!echoed) return ['fail', 'the session stopped working after the idle period']
  return ['ok', `2 idle intervals of ${seconds} s, PINGRESP each time, session still usable`]
}

async function runTransport(ctx, kind) {
  const emit = (id, status, note) => ctx.emit(id, status, note, kind)
  const opened = []
  const open = async (label, extra) => {
    const client = await connect(ctx, kind, label, extra)
    opened.push(client)
    return client
  }
  const retained = new Set()
  const base = (rest) => ctx.topic(`t/${ctx.run}/${rest}`)
  const guard = async (id, work) => {
    try {
      await work()
    } catch (error) {
      emit(id, 'fail', error.expected ? error.message : describeError(error))
    }
  }

  // 1. connect — nothing else can run without it.
  let sub
  try {
    const client = ctx.client(kind, 'sub')
    opened.push(client)
    await client.open()
    const code = await client.login(ctx.auth)
    if (code !== 0) throw loginError(code)
    sub = client
    emit('connect', 'ok', `open ${fmt(client.openMs)}, login ${fmt(client.connectMs)}`)
  } catch (error) {
    emit('connect', 'fail', error.expected ? error.message : describeError(error))
    for (const id of TRANSPORT_CHECKS.slice(1)) emit(id, 'skip', 'skipped: no working connection')
    for (const client of opened) client.kill()
    return
  }

  const keepalive = ctx.options.quick ? null : checkKeepalive(ctx, kind, open).catch((error) => ['fail', describeError(error)])
  if (ctx.options.quick) emit('keepalive', 'skip', 'skipped (--quick)')

  try {
    // 2. subscribe
    await guard('subscribe', async () => {
      const filters = [base('#'), ctx.topic('s/+')]
      const granted = []
      for (const filter of filters) granted.push(await sub.subscribe(filter))
      if (granted.every((g) => g === 1 || g === 2)) {
        emit('subscribe', 'ok', 'both filters granted at QoS 1 (own test subtree and the presence wildcard)')
      } else {
        emit('subscribe', 'fail', `own test subtree: ${grantedText(granted[0])}; presence wildcard ah/<user>/s/+: ${grantedText(granted[1])}`)
      }
    })

    // 3. roundtrip — a second session with the same credentials, like the Mac and the phone.
    const pub = await open('pub').catch((error) => {
      emit('roundtrip', 'fail', error.expected ? error.message : describeError(error))
      return null
    })
    if (pub) {
      await guard('roundtrip', async () => {
        const acks = []
        const delivered = []
        let missing = 0
        for (let i = 0; i < 15; i += 1) {
          const topic = base(`rt/${i}`)
          const payload = Buffer.from(JSON.stringify({ i, nonce: crypto.randomBytes(6).toString('hex') }))
          const started = process.hrtime.bigint()
          const ack = await pub.publish(topic, payload)
          if (ack !== null) acks.push(ack)
          const got = await sub.expectPublish((p) => p.topic === topic && p.payload.equals(payload), 8000)
          if (got) delivered.push(elapsedMs(started))
          else missing += 1
          if (pub.closed || sub.closed) break
        }
        if (missing === 0 && acks.length === 15) {
          const a = percentiles(acks)
          const d = percentiles(delivered)
          emit('roundtrip', 'ok', `15/15 delivered; PUBACK p50 ${fmt(a.p50)}, p95 ${fmt(a.p95)}; publish to delivery p50 ${fmt(d.p50)}, p95 ${fmt(d.p95)}`)
        } else {
          const why = pub.closed || sub.closed ? '; the broker closed a connection' : ''
          emit('roundtrip', 'fail', `${delivered.length}/15 delivered, ${acks.length}/15 acknowledged${why}`)
        }
      })

      // 4. retained
      await guard('retained', async () => {
        const topic = base('r')
        retained.add(topic)
        await pub.publish(topic, 'retained-1', { retain: true })
        const late = await open('late')
        await late.subscribe(topic)
        const got = await late.expectPublish((p) => p.topic === topic, 5000)
        late.close()
        if (!got) return emit('retained', 'fail', 'a subscriber that arrived later received nothing')
        if (!got.retain || got.payload.toString() !== 'retained-1') {
          return emit('retained', 'fail', 'the message arrived, but without the retain flag')
        }
        await pub.publish(topic, '', { retain: true })
        const after = await open('after')
        await after.subscribe(topic)
        const stale = await after.expectPublish((p) => p.topic === topic, ctx.options.settleMs)
        after.close()
        emit('retained', 'ok', stale ? 'delivered with the retain flag; an empty retained message did NOT clear it' : 'delivered with the retain flag; an empty retained message clears it')
      })
    } else {
      emit('retained', 'skip', 'skipped: no second session')
    }

    // 5. will (TCP drop) — the Mac's and the phone's "offline" signal.
    await guard('will', async () => {
      const topic = base('w')
      retained.add(topic)
      const watcher = await open('watch')
      await watcher.subscribe(topic)
      const victim = await open('victim', { keepalive: 30, will: { topic, payload: Buffer.from('offline'), qos: 1, retain: true } })
      await victim.publish(topic, 'online', { retain: true })
      await watcher.expectPublish((p) => p.topic === topic && p.payload.toString() === 'online', 5000)
      const started = process.hrtime.bigint()
      victim.kill()
      const gone = await watcher.expectPublish((p) => p.topic === topic && p.payload.toString() === 'offline', ctx.options.willWaitMs)
      const ms = elapsedMs(started)
      watcher.close()
      if (!gone) return emit('will', 'fail', `no will within ${Math.round(ctx.options.willWaitMs / 1000)} s of a dropped connection`)
      const later = await open('later-will')
      await later.subscribe(topic)
      const kept = await later.expectPublish((p) => p.topic === topic, 4000)
      later.close()
      if (!kept || !kept.retain || kept.payload.toString() !== 'offline') {
        return emit('will', 'fail', `the will arrived after ${fmt(ms)}, but was not kept as the retained message`)
      }
      emit('will', 'ok', `published ${fmt(ms)} after the drop and kept as retained`)
    })

    // 6. no will after a clean DISCONNECT
    await guard('will-clean', async () => {
      const topic = base('c')
      const watcher = await open('watch-clean')
      await watcher.subscribe(topic)
      const victim = await open('victim-clean', { will: { topic, payload: Buffer.from('offline'), qos: 1, retain: false } })
      victim.close()
      const spurious = await watcher.expectPublish((p) => p.topic === topic, Math.round(ctx.options.settleMs * 1.5))
      watcher.close()
      if (spurious) emit('will-clean', 'warn', 'the broker published the will although the client sent DISCONNECT (a duplicate "offline"; the apps announce their own first)')
      else emit('will-clean', 'ok', 'DISCONNECT did not trigger the will')
    })

    // 7. big messages
    await guard('big', async () => {
      const worked = []
      let failure = null
      for (const size of BIG_SIZES) {
        const topic = base(`big/${size}`)
        const payload = crypto.randomBytes(size)
        const sender = await open(`big-${size}`)
        const started = process.hrtime.bigint()
        const ack = await sender.publish(topic, payload, { timeoutMs: 30000 })
        const got = ack === null ? null : await sub.expectPublish((p) => p.topic === topic, 30000)
        const ms = elapsedMs(started)
        const intact = Boolean(got) && got.payload.equals(payload)
        sender.close()
        if (intact) {
          worked.push(`${size.toLocaleString('en-US')} B in ${fmt(ms)}`)
        } else {
          const reason = ack === null ? (sender.closed ? 'the broker closed the connection' : 'no PUBACK') : got ? 'the payload arrived changed' : 'not delivered'
          failure = `${size.toLocaleString('en-US')} B: ${reason}`
          break
        }
      }
      if (failure) emit('big', 'fail', `${failure}${worked.length ? `; worked up to ${worked[worked.length - 1]}` : ''}`)
      else emit('big', 'ok', worked.join('; '))
    })

    // 8. keepalive (started right after login, so the idle time overlaps the checks above)
    if (keepalive) {
      const [status, note] = await keepalive
      emit('keepalive', status, note)
    }
  } finally {
    // Leave nothing retained behind on the user's broker.
    try {
      if (retained.size > 0) {
        const cleaner = await connect(ctx, kind, 'clean')
        opened.push(cleaner)
        for (const topic of retained) await cleaner.publish(topic, '', { retain: true })
        cleaner.close()
      }
    } catch { /* the broker is unreachable now; nothing more can be done */ }
    for (const client of opened) if (!client.closed) client.kill()
  }
}

// ---------------------------------------------------------------------------- isolation

/**
 * One connection, one subscription filter, a set of publishes. Reports for each publish whether it was
 * delivered back — the only thing that matters. A refused subscription, a silently dropped message and a
 * closed connection are all "not delivered", and all count as isolated.
 */
async function isolationProbe(ctx, kind, spec) {
  const client = ctx.client(kind, 'iso')
  const outcome = { label: spec.label, targets: [] }
  try {
    await client.open()
    const code = await client.login(ctx.auth)
    if (code !== 0) throw loginError(code)
    const granted = await client.subscribe(spec.filter)
    outcome.refusedAtSubscribe = granted === 128
    for (const target of spec.publishes) {
      const tag = `iso-${ctx.run}-${crypto.randomBytes(4).toString('hex')}`
      const acked = client.closed ? null : await client.publish(target.topic, tag, { timeoutMs: 3000 })
      const got = client.closed ? null : await client.expectPublish((p) => p.payload.toString() === tag, ctx.options.settleMs)
      outcome.targets.push({ ...target, acked: acked !== null, delivered: Boolean(got) })
    }
    outcome.connectionClosed = client.closed
  } catch (error) {
    outcome.error = error.expected ? error.message : describeError(error)
  } finally {
    if (!client.closed) client.close()
  }
  return outcome
}

async function checkIsolation(ctx, kind) {
  const emit = (status, note) => ctx.emit('isolation', status, note, kind)
  const run = ctx.run
  const own = ctx.topic(`t/iso-${run}`)
  const other = `ah/o${run}/t/iso-${run}`
  const sibling = `ah/${ctx.user}x/t/iso-${run}`
  const outside = `iso-${run}/t`
  const specs = [
    { label: 'own topic', filter: own, publishes: [{ topic: own, foreign: false }] },
    { label: 'another user', filter: other, publishes: [{ topic: other, foreign: true }] },
    { label: 'a name that starts like ours', filter: sibling, publishes: [{ topic: sibling, foreign: true }] },
    { label: 'outside ah/', filter: outside, publishes: [{ topic: outside, foreign: true }] },
    {
      label: 'wildcard across users',
      filter: `ah/+/t/iso-${run}`,
      publishes: [{ topic: own, foreign: false }, { topic: other, foreign: true }],
    },
  ]
  try {
    const outcomes = await Promise.all(specs.map((spec) => isolationProbe(ctx, kind, spec)))
    const failed = outcomes.find((outcome) => outcome.error)
    if (failed) return emit('warn', `could not run the check: ${failed.error}`)
    const ownProbe = outcomes[0].targets[0]
    if (!ownProbe.delivered) return emit('warn', 'inconclusive: a message inside ah/<username>/ was not delivered back either')
    const leaks = []
    for (const outcome of outcomes) {
      for (const target of outcome.targets) if (target.foreign && target.delivered) leaks.push(outcome.label)
    }
    if (leaks.length > 0) {
      return emit('warn', `messages outside ah/<username>/ were delivered (${leaks.join(', ')}); anyone with this login could read or write other users' topics`)
    }
    const foreign = outcomes.slice(1)
    const refused = foreign.filter((outcome) => outcome.refusedAtSubscribe).length
    const closed = foreign.filter((outcome) => outcome.connectionClosed).length
    const how = refused + closed > 0
      ? `of ${foreign.length} probes, subscriptions refused: ${refused}, connections closed by the broker: ${closed}`
      : 'foreign messages are accepted, then dropped at delivery'
    emit('ok', `nothing outside ah/<username>/ was delivered (${how})`)
  } catch (error) {
    emit('warn', `could not run the check: ${describeError(error)}`)
  }
}

async function checkCrossUser(ctx, kind) {
  const emit = (status, note) => ctx.emit('cross-user', status, note, kind)
  const { user2, pass2 } = ctx.options
  const opened = []
  const session = async (label, credentials) => {
    const client = ctx.client(kind, label)
    opened.push(client)
    await client.open()
    const code = await client.login(credentials)
    if (code !== 0) throw loginError(code)
    return client
  }
  try {
    const first = ctx.auth
    const second = { user: user2, pass: pass2 }
    const aTopic = ctx.topic(`t/xu-${ctx.run}`)
    const bTopic = `ah/${user2}/t/xu-${ctx.run}`
    const tag = (name) => `xu-${name}-${crypto.randomBytes(4).toString('hex')}`
    const tags = { aOwn: tag('a-own'), aForeign: tag('a-foreign'), bOwn: tag('b-own'), bForeign: tag('b-foreign') }

    // Listeners only ever listen to their own topic. Writers publish to their own topic first (the positive
    // control), then to the other credential's topic; a broker may close a writer's connection for that.
    const listenerA = await session('xu-listen-a', first)
    const listenerB = await session('xu-listen-b', second)
    await listenerA.subscribe(aTopic)
    await listenerB.subscribe(bTopic)
    const writerA = await session('xu-write-a', first)
    const writerB = await session('xu-write-b', second)
    await writerA.publish(aTopic, tags.aOwn, { timeoutMs: 3000 })
    await writerB.publish(bTopic, tags.bOwn, { timeoutMs: 3000 })
    if (!writerA.closed) await writerA.publish(bTopic, tags.aForeign, { timeoutMs: 3000 })
    if (!writerB.closed) await writerB.publish(aTopic, tags.bForeign, { timeoutMs: 3000 })

    const hears = async (client, text, ms) => Boolean(await client.expectPublish((p) => p.payload.toString() === text, ms))
    const aOwn = await hears(listenerA, tags.aOwn, ctx.options.settleMs)
    const bOwn = await hears(listenerB, tags.bOwn, ctx.options.settleMs)
    if (!aOwn || !bOwn) return emit('warn', 'inconclusive: a session did not hear a message written to its own topic')
    const aHeardB = await hears(listenerA, tags.bForeign, ctx.options.settleMs)
    const bHeardA = await hears(listenerB, tags.aForeign, ctx.options.settleMs)
    if (aHeardB || bHeardA) {
      const who = []
      if (aHeardB) who.push('the second credential wrote into the first one\'s topic')
      if (bHeardA) who.push('the first credential wrote into the second one\'s topic')
      return emit('warn', `${who.join(' and ')}: the two logins are not separated`)
    }

    // Retained messages are served on subscribe, not published: the second credential's retained message
    // must not reach the first credential, not even through a wildcard.
    const retainedTopic = `ah/${user2}/t/xu-${ctx.run}-r`
    const retainedTag = tag('b-retained')
    let leakedRetained = false
    const retainedWriter = await session('xu-retain-b', second)
    try {
      await retainedWriter.publish(retainedTopic, retainedTag, { retain: true, timeoutMs: 3000 })
      const reader = await session('xu-read-a', first)
      await reader.subscribe(retainedTopic)
      await reader.subscribe(`ah/+/t/xu-${ctx.run}-r`)
      leakedRetained = await hears(reader, retainedTag, ctx.options.settleMs)
    } finally {
      if (!retainedWriter.closed) await retainedWriter.publish(retainedTopic, '', { retain: true, timeoutMs: 3000 })
    }
    if (leakedRetained) return emit('warn', 'the first credential received a retained message from the second one\'s namespace')
    emit('ok', 'each credential only reaches its own ah/<username>/ topics (live and retained messages)')
  } catch (error) {
    emit('warn', `could not run the check: ${error.expected ? error.message : describeError(error)}`)
  } finally {
    for (const client of opened) if (!client.closed) client.kill()
  }
}

// ---------------------------------------------------------------------------- slow checks

async function checkSilentWill(ctx, kind) {
  const emit = (note) => ctx.emit('silent-will', 'info', note, kind)
  const seconds = ctx.options.slowKeepaliveSecs
  const topic = ctx.topic(`t/${ctx.run}/sw`)
  const opened = []
  try {
    const watcher = await connect(ctx, kind, 'watch-silent')
    opened.push(watcher)
    await watcher.subscribe(topic)
    const victim = await connect(ctx, kind, 'victim-silent', {
      keepalive: seconds,
      autoPing: false,
      will: { topic, payload: Buffer.from('offline'), qos: 1, retain: false },
    })
    opened.push(victim)
    const started = process.hrtime.bigint()
    // The victim now says nothing at all; the broker has to notice on its own.
    const limitMs = (4 * seconds + 10) * 1000
    const got = await watcher.expectPublish((p) => p.topic === topic, limitMs)
    if (!got && watcher.closed) return emit('could not judge: the broker closed the watching session first')
    if (!got) return emit(`no will within ${Math.round(limitMs / 1000)} s of a silent connection (keepalive ${seconds} s): a half-dead connection is not noticed in a useful time`)
    const ms = elapsedMs(started)
    emit(`the will arrived after ${(ms / 1000).toFixed(1)} s, ${(ms / 1000 / seconds).toFixed(1)}x the keepalive of ${seconds} s (MQTT allows 1.5x)`)
  } catch (error) {
    emit(`could not run the check: ${error.expected ? error.message : describeError(error)}`)
  } finally {
    for (const client of opened) client.kill()
  }
}

async function checkWsCloseWill(ctx, kind) {
  const emit = (note) => ctx.emit('ws-close-will', 'info', note, kind)
  const topic = ctx.topic(`t/${ctx.run}/wc`)
  const opened = []
  try {
    const watcher = await connect(ctx, kind, 'watch-wsclose')
    opened.push(watcher)
    await watcher.subscribe(topic)
    const victim = await connect(ctx, kind, 'victim-wsclose', {
      will: { topic, payload: Buffer.from('offline'), qos: 1, retain: false },
    })
    opened.push(victim)
    const started = process.hrtime.bigint()
    victim.pipe.wsClose()
    const got = await watcher.expectPublish((p) => p.topic === topic, 10000)
    if (got) emit(`the will arrived ${fmt(elapsedMs(started))} after the WebSocket close frame`)
    else emit('no will within 10 s of a WebSocket close frame without DISCONNECT (harmless: the apps send DISCONNECT first)')
  } catch (error) {
    emit(`could not run the check: ${error.expected ? error.message : describeError(error)}`)
  } finally {
    for (const client of opened) client.kill()
  }
}

// ---------------------------------------------------------------------------- the run

/**
 * options:
 *   host, tlsPort, wsPort, wsPath, user, pass, insecureLocal
 *   kinds        transports to check, in order, e.g. ['tls', 'wss'] (or ['tcp', 'ws'] with insecureLocal)
 *   user2, pass2 a second credential for the cross-user check (optional)
 *   quick, slow  skip the idle keepalive check / add the silent-loss measurements
 *   only         'isolation' runs just the login and the isolation checks (first transport only)
 *   keepaliveSecs, slowKeepaliveSecs, willWaitMs, timeoutMs, run, onResult
 */
async function runCompat(options) {
  const ctx = new Context(options)
  const kinds = options.kinds
  if (ctx.options.only === 'isolation') {
    const kind = kinds[0]
    let client = null
    try {
      client = await connect(ctx, kind, 'login')
      ctx.emit('connect', 'ok', `open ${fmt(client.openMs)}, login ${fmt(client.connectMs)}`, kind)
      client.close()
    } catch (error) {
      ctx.emit('connect', 'fail', error.expected ? error.message : describeError(error), kind)
    }
    if (client) {
      await checkIsolation(ctx, kind)
      if (ctx.options.user2) await checkCrossUser(ctx, kind)
    }
    return { run: ctx.run, results: ctx.results }
  }
  for (const kind of kinds) await runTransport(ctx, kind)
  const reachable = kinds.find((kind) => ctx.results.some((r) => r.transport === kind && r.id === 'connect' && r.status === 'ok'))
  if (reachable) {
    await checkIsolation(ctx, reachable)
    if (ctx.options.user2) await checkCrossUser(ctx, reachable)
    if (ctx.options.slow) {
      await checkSilentWill(ctx, reachable)
      const ws = kinds.find((kind) => (kind === 'wss' || kind === 'ws') && ctx.results.some((r) => r.transport === kind && r.id === 'connect' && r.status === 'ok'))
      if (ws) await checkWsCloseWill(ctx, ws)
    }
  } else {
    ctx.emit('isolation', 'skip', 'skipped: no working connection')
  }
  return { run: ctx.run, results: ctx.results }
}

module.exports = { runCompat, CHECKS, TRANSPORT_CHECKS, BIG_SIZES, describeError, percentiles }
