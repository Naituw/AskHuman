#!/usr/bin/env node
'use strict'

// Reachability and latency probe for the MQTT broker behind the iOS channel.
//
// It needs no account and no password: every round opens a fresh connection, sends an MQTT CONNECT
// without credentials and times how long the broker takes to answer (it refuses, which is all this
// needs). Nothing is published, nothing is subscribed, and nothing is stored.
//
//   node probe.cjs --host <host> [--port 8883] [--ws-port 8084] [--rounds 20] [--json]
//
// What each line means:
//   tls  TCP + TLS handshake + CONNECT/CONNACK over MQTT on --port (what the Mac and the phone use first)
//   wss  TCP + TLS + WebSocket upgrade + CONNECT/CONNACK on --ws-port (the fallback for networks that
//        only let web traffic through)
//
// Run it from the network you care about (home, office, mobile hotspot). Zero dependencies; Node 22+
// (it uses the built-in WebSocket client).

const tls = require('node:tls')
const crypto = require('node:crypto')
const { performance } = require('node:perf_hooks')

function parseArgs(argv) {
  const options = { host: '', port: 8883, wsPort: 8084, rounds: 20, json: false, timeoutMs: 8000, gapMs: 400 }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--host') options.host = argv[++i] || ''
    else if (arg === '--port') options.port = Number(argv[++i])
    else if (arg === '--ws-port') options.wsPort = Number(argv[++i])
    else if (arg === '--rounds') options.rounds = Number(argv[++i])
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--json') options.json = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (!options.host) throw new Error('--host is required')
  if (!Number.isInteger(options.rounds) || options.rounds < 1) throw new Error('--rounds must be a positive integer')
  return options
}

/** An MQTT 3.1.1 CONNECT with a random client id and no credentials. */
function connectPacket() {
  const clientId = Buffer.from(`ah-probe-${crypto.randomBytes(6).toString('hex')}`)
  const variable = Buffer.concat([
    Buffer.from([0x00, 0x04, 0x4d, 0x51, 0x54, 0x54, 0x04, 0x02, 0x00, 0x1e]),
    Buffer.from([clientId.length >> 8, clientId.length & 0xff]),
    clientId,
  ])
  return Buffer.concat([Buffer.from([0x10, variable.length]), variable])
}

const REFUSALS = {
  0: 'accepted',
  1: 'unacceptable protocol version',
  2: 'identifier rejected',
  3: 'server unavailable',
  4: 'bad user name or password',
  5: 'not authorized',
}

function probeTls(host, port, timeoutMs) {
  return new Promise((resolve) => {
    const start = performance.now()
    const marks = {}
    let settled = false
    const socket = tls.connect({ host, port, servername: host })
    const finish = (result) => {
      if (settled) return
      settled = true
      socket.destroy()
      resolve(result)
    }
    const timer = setTimeout(() => finish({ ok: false, error: `timed out after ${timeoutMs} ms` }), timeoutMs)
    socket.once('lookup', () => { marks.dns = performance.now() - start })
    socket.once('connect', () => { marks.tcp = performance.now() - start })
    socket.once('secureConnect', () => {
      marks.tls = performance.now() - start
      marks.sent = performance.now()
      socket.write(connectPacket())
    })
    socket.on('data', (data) => {
      if (data[0] !== 0x20 || data.length < 4) return
      clearTimeout(timer)
      const total = performance.now() - start
      finish({
        ok: true,
        total,
        handshake: marks.tls,
        connack: performance.now() - marks.sent,
        answer: REFUSALS[data[3]] || `code ${data[3]}`,
        protocol: socket.getProtocol(),
      })
    })
    socket.on('error', (error) => { clearTimeout(timer); finish({ ok: false, error: error.code || error.message }) })
    socket.on('close', () => { clearTimeout(timer); finish({ ok: false, error: 'closed before an answer' }) })
  })
}

function probeWss(host, port, timeoutMs) {
  return new Promise((resolve) => {
    const start = performance.now()
    let opened = 0
    let sent = 0
    let settled = false
    let ws
    const finish = (result) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      try { ws.close() } catch { /* already closed */ }
      resolve(result)
    }
    const timer = setTimeout(() => finish({ ok: false, error: `timed out after ${timeoutMs} ms` }), timeoutMs)
    try {
      ws = new WebSocket(`wss://${host}:${port}/mqtt`, 'mqtt')
    } catch (error) {
      finish({ ok: false, error: error.message })
      return
    }
    ws.binaryType = 'arraybuffer'
    ws.addEventListener('open', () => {
      opened = performance.now() - start
      sent = performance.now()
      ws.send(connectPacket())
    })
    ws.addEventListener('message', (event) => {
      const data = Buffer.from(event.data)
      if (data[0] !== 0x20 || data.length < 4) return
      finish({
        ok: true,
        total: performance.now() - start,
        handshake: opened,
        connack: performance.now() - sent,
        answer: REFUSALS[data[3]] || `code ${data[3]}`,
      })
    })
    ws.addEventListener('error', () => finish({ ok: false, error: 'connection failed' }))
    ws.addEventListener('close', () => finish({ ok: false, error: 'closed before an answer' }))
  })
}

function percentile(sorted, p) {
  if (sorted.length === 0) return null
  const index = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1)
  return sorted[Math.max(0, index)]
}

function summarize(results) {
  const good = results.filter((result) => result.ok)
  const stat = (key) => {
    const values = good.map((result) => result[key]).sort((a, b) => a - b)
    return {
      p50: round(percentile(values, 50)),
      p95: round(percentile(values, 95)),
      max: round(values[values.length - 1] ?? null),
    }
  }
  const errors = {}
  for (const result of results.filter((entry) => !entry.ok)) errors[result.error] = (errors[result.error] || 0) + 1
  return {
    attempts: results.length,
    succeeded: good.length,
    samples: results.map((result) =>
      result.ok
        ? { total: round(result.total), handshake: round(result.handshake), connack: round(result.connack) }
        : { error: result.error },
    ),
    total: stat('total'),
    handshake: stat('handshake'),
    connack: stat('connack'),
    answers: [...new Set(good.map((result) => result.answer))],
    errors,
  }
}

const round = (value) => (value === null || value === undefined ? null : Math.round(value))
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function run(label, rounds, gapMs, once) {
  const results = []
  for (let i = 0; i < rounds; i += 1) {
    results.push(await once())
    if (i + 1 < rounds) await sleep(gapMs)
  }
  return { label, ...summarize(results) }
}

function describe(report) {
  // The per-attempt samples are for --json only.
  const ms = (value) => (value === null ? '-' : `${value} ms`)
  const lines = [`${report.label}: ${report.succeeded}/${report.attempts} answered`]
  if (report.succeeded > 0) {
    lines.push(`  whole exchange   p50 ${ms(report.total.p50)}  p95 ${ms(report.total.p95)}  max ${ms(report.total.max)}`)
    lines.push(`  until connected  p50 ${ms(report.handshake.p50)}  p95 ${ms(report.handshake.p95)}`)
    lines.push(`  CONNECT → reply  p50 ${ms(report.connack.p50)}  p95 ${ms(report.connack.p95)}  (one network round trip)`)
    lines.push(`  the broker said  ${report.answers.join(', ')}`)
  }
  for (const [error, count] of Object.entries(report.errors)) lines.push(`  failed ${count}×: ${error}`)
  return lines.join('\n')
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const reports = [
    await run(`tls ${options.host}:${options.port}`, options.rounds, options.gapMs, () => probeTls(options.host, options.port, options.timeoutMs)),
    await run(`wss ${options.host}:${options.wsPort}`, options.rounds, options.gapMs, () => probeWss(options.host, options.wsPort, options.timeoutMs)),
  ]
  if (options.json) {
    process.stdout.write(`${JSON.stringify({ host: options.host, when: new Date().toISOString(), reports }, null, 2)}\n`)
  } else {
    process.stdout.write(`${reports.map(describe).join('\n\n')}\n`)
  }
  process.exit(reports.every((report) => report.succeeded > 0) ? 0 : 1)
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`)
  process.exit(2)
})
