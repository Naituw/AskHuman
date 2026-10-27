'use strict'

// A local MQTT broker that behaves like the production one as far as the iOS channel cares:
// username/password login, retained messages, last wills, and the access rule
//   "everyone may publish and subscribe to ah/<own username>/#, nothing else".
// Plain TCP on loopback only: this is for tests and demos, never a stand-in for EMQX.
//
//   node broker.js [--port 18830] [--host 127.0.0.1] --user alice:secret [--user bob:other] [--verbose]
//   AH_BROKER_USERS="alice:secret,bob:other" node broker.js
//
// Once it listens it prints one line, `READY {"port":<n>}`, on stdout.

const net = require('node:net')
const { Aedes } = require('aedes')

function parseArgs(argv) {
  const options = { host: '127.0.0.1', port: 18830, users: new Map(), verbose: false }
  const addUser = (text) => {
    const at = text.indexOf(':')
    if (at <= 0) throw new Error(`bad --user value (expected name:password)`)
    options.users.set(text.slice(0, at), text.slice(at + 1))
  }
  for (const entry of (process.env.AH_BROKER_USERS || '').split(',')) {
    if (entry.trim()) addUser(entry.trim())
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--port') options.port = Number(argv[++i])
    else if (arg === '--host') options.host = argv[++i]
    else if (arg === '--user') addUser(argv[++i] || '')
    else if (arg === '--verbose') options.verbose = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (options.users.size === 0) throw new Error('no users: pass --user name:password or set AH_BROKER_USERS')
  if (!['127.0.0.1', 'localhost', '::1'].includes(options.host)) {
    throw new Error('this broker only listens on loopback addresses')
  }
  return options
}

/** The access rule: the topic (or filter) must live under `ah/<username>/`. */
function allowed(username, topic) {
  if (!username) return false
  return topic.startsWith(`ah/${username}/`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const aedes = await Aedes.createBroker({ maxClientsIdLength: 256 })
  const log = (...parts) => {
    if (options.verbose) process.stderr.write(`${new Date().toISOString()} ${parts.join(' ')}\n`)
  }

  aedes.authenticate = (client, username, password, callback) => {
    const expected = options.users.get(username || '')
    if (expected !== undefined && password && password.toString() === expected) {
      client.ahUser = username
      callback(null, true)
      return
    }
    const error = new Error('bad credentials')
    error.returnCode = 4
    callback(error, null)
  }

  aedes.authorizePublish = (client, packet, callback) => {
    if (client && !allowed(client.ahUser, packet.topic)) {
      log('refused publish', client.id, packet.topic)
      callback(new Error('publish refused'))
      return
    }
    callback(null)
  }

  aedes.authorizeSubscribe = (client, subscription, callback) => {
    if (!allowed(client.ahUser, subscription.topic)) {
      log('refused subscribe', client.id, subscription.topic)
      callback(null, null)
      return
    }
    callback(null, subscription)
  }

  aedes.on('clientReady', (client) => log('connected', client.id, client.ahUser))
  aedes.on('clientDisconnect', (client) => log('disconnected', client.id))

  const server = net.createServer(aedes.handle)
  server.listen(options.port, options.host, () => {
    process.stdout.write(`READY ${JSON.stringify({ port: server.address().port })}\n`)
  })

  const stop = () => {
    server.close()
    aedes.close(() => process.exit(0))
    setTimeout(() => process.exit(0), 1500).unref()
  }
  process.on('SIGTERM', stop)
  process.on('SIGINT', stop)
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`)
  process.exit(1)
})
