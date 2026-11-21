#!/usr/bin/env node
'use strict'

// Compatibility suite for the MQTT broker behind the AskHuman iOS channel.
//
// Run it against any broker before pointing the apps at it: it checks, over the same transports the
// apps use, everything the channel needs from a broker (see docs/wiki/ios-broker.md), and ends with
// COMPATIBLE or NOT COMPATIBLE. The password is read from the environment, never from an argument,
// and is never printed.
//
//   AH_PROBE_PASSWORD=... node compat.cjs --host broker.example.com --user alice
//   AH_PROBE_PASSWORD=... node compat.cjs --host broker.example.com --user alice --ws-port 443 --slow --markdown
//
// Exit status: 0 compatible, 1 a required check failed, 2 wrong usage.

const { runCompat } = require('./lib/checks.cjs')
const { line, textSummary, toJson, toMarkdown } = require('./lib/report.cjs')

const USAGE = `usage: AH_PROBE_PASSWORD=... node compat.cjs --host <broker host> --user <username> [options]

  --host H                broker host name (required)
  --user NAME             broker user name (required; for flespi, the token id)
  --port N                MQTT over TLS port (default 8883; 1883 with --insecure-local)
  --ws-port N             MQTT over WSS port; 0 skips the WebSocket checks (default 0)
  --ws-path PATH          WebSocket path (default /mqtt)
  --password-env NAME     environment variable holding the password (default AH_PROBE_PASSWORD)
  --user2 NAME            a second credential, for the cross-user isolation check (optional)
  --password2-env NAME    environment variable holding its password (default AH_PROBE_PASSWORD2)
  --quick                 skip the idle-time keepalive check (saves about 20 s per transport)
  --slow                  also measure the silent-loss will and the WebSocket close (about 2 minutes)
  --only isolation        run just the login and the isolation checks (what acl-check.cjs does)
  --strict                exit with 1 on warnings as well (an isolation leak is a warning)
  --keepalive-secs N      keepalive of the idle check (default 10)
  --insecure-local        plain TCP / WebSocket to a broker on this machine (for tests; loopback only)
  --label TEXT            name used in the report instead of the host name
  --json                  print the report as JSON after the run
  --markdown              print the report as a Markdown table after the run
`

function parseArgs(argv, env) {
  const o = {
    host: null, user: null, tlsPort: null, wsPort: 0, wsPath: '/mqtt', passwordEnv: 'AH_PROBE_PASSWORD',
    user2: null, password2Env: 'AH_PROBE_PASSWORD2', quick: false, slow: false, strict: false, only: null, keepaliveSecs: 10,
    insecureLocal: false, label: null, json: false, markdown: false,
  }
  const value = (i, name) => {
    if (i + 1 >= argv.length) throw new Error(`${name} needs a value`)
    return argv[i + 1]
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--host') o.host = value(i++, arg)
    else if (arg === '--user') o.user = value(i++, arg)
    else if (arg === '--port') o.tlsPort = Number(value(i++, arg))
    else if (arg === '--ws-port') o.wsPort = Number(value(i++, arg))
    else if (arg === '--ws-path') o.wsPath = value(i++, arg)
    else if (arg === '--password-env') o.passwordEnv = value(i++, arg)
    else if (arg === '--user2') o.user2 = value(i++, arg)
    else if (arg === '--password2-env') o.password2Env = value(i++, arg)
    else if (arg === '--keepalive-secs') o.keepaliveSecs = Number(value(i++, arg))
    else if (arg === '--label') o.label = value(i++, arg)
    else if (arg === '--quick') o.quick = true
    else if (arg === '--slow') o.slow = true
    else if (arg === '--strict') o.strict = true
    else if (arg === '--only') o.only = value(i++, arg)
    else if (arg === '--insecure-local') o.insecureLocal = true
    else if (arg === '--json') o.json = true
    else if (arg === '--markdown') o.markdown = true
    else if (arg === '--help' || arg === '-h') o.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (o.help) return o
  if (!o.host) throw new Error('--host is required')
  if (!o.user) throw new Error('--user is required')
  if (o.tlsPort === null) o.tlsPort = o.insecureLocal ? 1883 : 8883
  for (const [name, n] of [['--port', o.tlsPort], ['--ws-port', o.wsPort], ['--keepalive-secs', o.keepaliveSecs]]) {
    if (!Number.isInteger(n) || n < 0 || n > 65535) throw new Error(`${name} must be a whole number`)
  }
  if (o.keepaliveSecs < 1) throw new Error('--keepalive-secs must be at least 1')
  if (o.only !== null && o.only !== 'isolation') throw new Error('--only accepts: isolation')
  o.pass = env[o.passwordEnv]
  if (!o.pass) throw new Error(`the password is read from the environment variable ${o.passwordEnv}; it is not set`)
  if (o.user2) {
    o.pass2 = env[o.password2Env]
    if (!o.pass2) throw new Error(`the second password is read from ${o.password2Env}; it is not set`)
  }
  return o
}

async function main() {
  let options
  try {
    options = parseArgs(process.argv.slice(2), process.env)
  } catch (error) {
    process.stderr.write(`${error.message}\n\n${USAGE}`)
    process.exitCode = 2
    return
  }
  if (options.help) {
    process.stdout.write(USAGE)
    return
  }

  const plain = options.insecureLocal
  const kinds = [plain ? 'tcp' : 'tls']
  if (options.wsPort > 0) kinds.push(plain ? 'ws' : 'wss')

  const log = (text) => process.stderr.write(`${text}\n`)
  log(`Checking ${options.label || options.host} over ${kinds.join(' and ')}${options.slow ? ' (including the slow checks)' : ''} ...`)
  const { results } = await runCompat({
    ...options,
    kinds,
    onResult: (result) => log(line(result)),
  })

  const meta = {
    label: options.label || options.host,
    date: new Date().toISOString().slice(0, 10),
    node: process.versions.node,
    transports: kinds,
  }
  if (options.json) process.stdout.write(`${toJson(meta, results)}\n`)
  if (options.markdown) process.stdout.write(`${toMarkdown(meta, results)}\n`)
  log(textSummary(results, { only: options.only, strict: options.strict }))
  // Let buffered output drain; the timer only fires if something unexpected keeps the process alive.
  const bad = (r) => r.status === 'fail' || (options.strict && r.status === 'warn')
  const code = results.some(bad) ? 1 : 0
  process.exitCode = code
  setTimeout(() => process.exit(code), 3000).unref()
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`)
  process.exitCode = 2
})
