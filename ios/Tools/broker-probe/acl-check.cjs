#!/usr/bin/env node
'use strict'

// Access-rule check for the MQTT broker behind the iOS channel.
//
// The channel shares one broker account between a Mac and a phone and relies on the broker's access
// rule to keep different accounts apart: an account may touch only ah/<its own username>/…
// This script logs in as ONE account and tries to step outside that namespace. It passes when the
// account can use its own namespace and every attempt outside it is refused.
//
//   AH_PROBE_PASSWORD=… node acl-check.cjs --host <host> --user <username> --other <another username>
//       [--port 8883] [--protocol 5|4] [--insecure-local]
//
// - --other names the account whose namespace is attacked; it does not need to exist.
// - The password comes from AH_PROBE_PASSWORD only: never from the command line, never printed.
// - Use MQTT 5 (the default) against EMQX: a refused publish is visible there as a PUBACK reason
//   code. MQTT 3.1.1 (--protocol 4) cannot show it, so those checks come out "inconclusive".
// - Everything it publishes goes under ah/<user>/probe/ or ah/<other>/probe/ and carries no data.
//
// Needs `npm install` in this directory (mqtt-packet).

const net = require('node:net')
const tls = require('node:tls')
const crypto = require('node:crypto')
const mqttPacket = require('mqtt-packet')

function parseArgs(argv) {
  const options = { host: '', port: 8883, user: '', other: '', protocol: 5, insecureLocal: false, timeoutMs: 8000 }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--host') options.host = argv[++i] || ''
    else if (arg === '--port') options.port = Number(argv[++i])
    else if (arg === '--user') options.user = argv[++i] || ''
    else if (arg === '--other') options.other = argv[++i] || ''
    else if (arg === '--protocol') options.protocol = Number(argv[++i])
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--insecure-local') options.insecureLocal = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  for (const key of ['host', 'user', 'other']) {
    if (!options[key]) throw new Error(`--${key} is required`)
  }
  if (![4, 5].includes(options.protocol)) throw new Error('--protocol must be 4 or 5')
  if (options.insecureLocal && !['127.0.0.1', 'localhost', '::1'].includes(options.host)) {
    throw new Error('--insecure-local only works with loopback hosts')
  }
  if (!process.env.AH_PROBE_PASSWORD) throw new Error('set AH_PROBE_PASSWORD (the account password)')
  return options
}

/** One logged-in MQTT connection with a tiny "wait for this packet" helper. */
function connect(options) {
  return new Promise((resolve, reject) => {
    const socket = options.insecureLocal
      ? net.connect({ host: options.host, port: options.port })
      : tls.connect({ host: options.host, port: options.port, servername: options.host })
    const parser = mqttPacket.parser({ protocolVersion: options.protocol })
    const inbox = []
    const waiters = []
    let closed = false
    let nextId = 1

    const session = {
      closed: () => closed,
      send(packet) {
        socket.write(mqttPacket.generate(packet, { protocolVersion: options.protocol }))
      },
      id: () => nextId++,
      /** Resolves with the first matching packet, or `null` when the connection closes or time runs out. */
      expect(match, ms = options.timeoutMs) {
        const index = inbox.findIndex(match)
        if (index >= 0) return Promise.resolve(inbox.splice(index, 1)[0])
        if (closed) return Promise.resolve(null)
        return new Promise((done) => {
          const waiter = { match, done }
          waiter.timer = setTimeout(() => {
            waiters.splice(waiters.indexOf(waiter), 1)
            done(null)
          }, ms)
          waiters.push(waiter)
        })
      },
      close() {
        try { session.send({ cmd: 'disconnect' }) } catch { /* the socket is already gone */ }
        socket.destroy()
      },
    }

    parser.on('packet', (packet) => {
      const index = waiters.findIndex((waiter) => waiter.match(packet))
      if (index >= 0) {
        const [waiter] = waiters.splice(index, 1)
        clearTimeout(waiter.timer)
        waiter.done(packet)
      } else {
        inbox.push(packet)
      }
    })
    parser.on('error', () => socket.destroy())
    socket.on('data', (data) => parser.parse(data))
    const onClosed = () => {
      closed = true
      for (const waiter of waiters.splice(0)) {
        clearTimeout(waiter.timer)
        waiter.done(null)
      }
    }
    socket.on('close', onClosed)
    socket.on('error', (error) => {
      onClosed()
      reject(error)
    })

    socket.once(options.insecureLocal ? 'connect' : 'secureConnect', async () => {
      session.send({
        cmd: 'connect',
        protocolId: 'MQTT',
        protocolVersion: options.protocol,
        clean: true,
        clientId: `ah-acl-${crypto.randomBytes(5).toString('hex')}`,
        keepalive: 30,
        username: options.user,
        password: Buffer.from(process.env.AH_PROBE_PASSWORD),
      })
      const connack = await session.expect((packet) => packet.cmd === 'connack')
      if (!connack) return reject(new Error('the broker closed the connection during login'))
      const code = options.protocol === 5 ? connack.reasonCode : connack.returnCode
      if (code !== 0) return reject(new Error(`login refused (code ${code}): wrong user name or password?`))
      resolve(session)
    })
  })
}

const REFUSED = 'refused'
const ALLOWED = 'ALLOWED'
const UNKNOWN = 'inconclusive'

/** Subscribe on a fresh connection. A SUBACK failure code, or a dropped connection, means refused. */
async function trySubscribe(options, filter) {
  const session = await connect(options)
  const id = session.id()
  session.send({
    cmd: 'subscribe',
    messageId: id,
    subscriptions: [{ topic: filter, qos: 1, nl: false, rap: false, rh: 0 }],
  })
  const ack = await session.expect((packet) => packet.cmd === 'suback' && packet.messageId === id)
  session.close()
  if (!ack) return { verdict: REFUSED, note: 'the broker closed the connection' }
  const code = ack.granted?.[0]
  return code === undefined || code >= 0x80
    ? { verdict: REFUSED, note: `SUBACK 0x${(code ?? 0x80).toString(16)}` }
    : { verdict: ALLOWED, note: `SUBACK granted QoS ${code}` }
}

/** Publish QoS 1 on a fresh connection. MQTT 5 shows a refusal as a PUBACK reason code. */
async function tryPublish(options, topic) {
  const session = await connect(options)
  const id = session.id()
  session.send({ cmd: 'publish', topic, payload: Buffer.alloc(0), qos: 1, retain: false, dup: false, messageId: id })
  const ack = await session.expect((packet) => packet.cmd === 'puback' && packet.messageId === id)
  session.close()
  if (!ack) return { verdict: REFUSED, note: 'the broker closed the connection' }
  if (options.protocol === 5) {
    const code = ack.reasonCode ?? 0
    return code >= 0x80
      ? { verdict: REFUSED, note: `PUBACK 0x${code.toString(16)}` }
      : { verdict: ALLOWED, note: 'PUBACK accepted' }
  }
  return { verdict: UNKNOWN, note: 'MQTT 3.1.1 acknowledges a dropped message too' }
}

/** Subscribe, publish to the same topic and wait for the message to come back: the allowed path. */
async function tryOwnRoundTrip(options, topic) {
  const session = await connect(options)
  const subId = session.id()
  session.send({
    cmd: 'subscribe',
    messageId: subId,
    subscriptions: [{ topic, qos: 1, nl: false, rap: false, rh: 0 }],
  })
  const suback = await session.expect((packet) => packet.cmd === 'suback' && packet.messageId === subId)
  const granted = suback?.granted?.[0]
  if (granted === undefined || granted >= 0x80) {
    session.close()
    return { verdict: REFUSED, note: `subscribe refused (0x${(granted ?? 0x80).toString(16)})` }
  }
  const pubId = session.id()
  session.send({ cmd: 'publish', topic, payload: Buffer.from('probe'), qos: 1, retain: false, dup: false, messageId: pubId })
  const echoed = await session.expect((packet) => packet.cmd === 'publish' && packet.topic === topic)
  session.close()
  return echoed
    ? { verdict: ALLOWED, note: 'published and received it back' }
    : { verdict: REFUSED, note: 'the message never came back' }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const { user, other } = options
  const tag = crypto.randomBytes(3).toString('hex')

  // Fail once, clearly, when the login itself does not work (wrong host, port, user or password).
  const first = await connect(options)
  first.close()

  // [what, expected verdict, how to try]
  const checks = [
    ['own namespace: subscribe + publish + receive', ALLOWED, () => tryOwnRoundTrip(options, `ah/${user}/probe/${tag}`)],
    ['own namespace: the presence wildcard the app uses', ALLOWED, () => trySubscribe(options, `ah/${user}/s/+`)],
    ['another account: subscribe to everything under it', REFUSED, () => trySubscribe(options, `ah/${other}/#`)],
    ['another account: subscribe to one topic', REFUSED, () => trySubscribe(options, `ah/${other}/d/probe-${tag}`)],
    ['another account: publish one message', REFUSED, () => tryPublish(options, `ah/${other}/probe/${tag}`)],
    ['wildcard over every account (ah/+/…)', REFUSED, () => trySubscribe(options, `ah/+/d/probe-${tag}`)],
    ['the whole ah/ tree (ah/#)', REFUSED, () => trySubscribe(options, 'ah/#')],
    ['everything (#)', REFUSED, () => trySubscribe(options, '#')],
    ['broker internals ($SYS/#)', REFUSED, () => trySubscribe(options, '$SYS/#')],
    ['a topic outside ah/', REFUSED, () => trySubscribe(options, `other/${tag}`)],
    ['an account whose name starts with ours', REFUSED, () => trySubscribe(options, `ah/${user}x/probe-${tag}`)],
    ['a publish to that sibling account', REFUSED, () => tryPublish(options, `ah/${user}x/probe/${tag}`)],
  ]

  let failed = 0
  let inconclusive = 0
  for (const [what, expected, run] of checks) {
    let result
    try {
      result = await run()
    } catch (error) {
      result = { verdict: 'ERROR', note: error.message }
    }
    const ok = result.verdict === expected
    const unknown = result.verdict === UNKNOWN
    if (unknown) inconclusive += 1
    else if (!ok) failed += 1
    const mark = ok ? 'ok  ' : unknown ? 'n/a ' : 'FAIL'
    process.stdout.write(`${mark} ${what}: ${result.verdict} (${result.note})\n`)
  }
  process.stdout.write(
    failed === 0
      ? `\nPASS: the account stays inside ah/${user}/` + (inconclusive ? ` (${inconclusive} check(s) inconclusive on MQTT 3.1.1; rerun with --protocol 5)` : '') + '\n'
      : `\nFAIL: ${failed} check(s) did not behave as the access rule requires.\n`,
  )
  process.exit(failed === 0 ? 0 : 1)
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`)
  process.exit(2)
})
