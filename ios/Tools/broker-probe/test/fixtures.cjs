'use strict'

// Brokers for the suite's own tests: a real (Aedes) broker on loopback, configurable to behave like the
// good brokers and like each way a bad one fails. Plain TCP and a plain WebSocket front, nothing else.

const net = require('node:net')
const crypto = require('node:crypto')
const { Duplex } = require('node:stream')
const { Aedes } = require('aedes')

const inside = (user, topic) => Boolean(user) && topic.startsWith(`ah/${user}/`)

/** One server-to-client WebSocket frame (not masked). */
function serverFrame(opcode, payload) {
  const length = payload.length
  let header
  if (length < 126) header = Buffer.from([0x80 | opcode, length])
  else if (length < 65536) {
    header = Buffer.alloc(4)
    header[0] = 0x80 | opcode
    header[1] = 126
    header.writeUInt16BE(length, 2)
  } else {
    header = Buffer.alloc(10)
    header[0] = 0x80 | opcode
    header[1] = 127
    header.writeBigUInt64BE(BigInt(length), 2)
  }
  return Buffer.concat([header, payload])
}

/** A WebSocket (RFC 6455) server that hands each connection's binary payloads to `handle` as a duplex stream. */
function webSocketServer(handle) {
  return net.createServer((socket) => {
    let head = Buffer.alloc(0)
    let pending = Buffer.alloc(0)
    let upgraded = false
    let duplex = null
    socket.on('error', () => {})
    socket.on('close', () => duplex && duplex.destroy())
    socket.on('data', (data) => {
      if (!upgraded) {
        head = Buffer.concat([head, data])
        const end = head.indexOf('\r\n\r\n')
        if (end < 0) return
        const text = head.subarray(0, end).toString()
        const key = /Sec-WebSocket-Key: (.+)/i.exec(text)
        if (!key) return socket.destroy()
        const accept = crypto.createHash('sha1').update(`${key[1].trim()}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64')
        socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\nSec-WebSocket-Protocol: mqtt\r\n\r\n`)
        upgraded = true
        duplex = new Duplex({
          read() {},
          write(chunk, _encoding, done) {
            socket.write(serverFrame(0x2, chunk))
            done()
          },
          final(done) {
            socket.end()
            done()
          },
        })
        duplex.on('close', () => socket.destroy())
        handle(duplex)
        data = head.subarray(end + 4)
        head = Buffer.alloc(0)
      }
      pending = Buffer.concat([pending, data])
      for (;;) {
        if (pending.length < 2) return
        const opcode = pending[0] & 0x0f
        let length = pending[1] & 0x7f
        let offset = 2
        if (length === 126) {
          if (pending.length < 4) return
          length = pending.readUInt16BE(2)
          offset = 4
        } else if (length === 127) {
          if (pending.length < 10) return
          length = Number(pending.readBigUInt64BE(2))
          offset = 10
        }
        if (pending.length < offset + 4 + length) return
        const mask = pending.subarray(offset, offset + 4)
        const payload = Buffer.from(pending.subarray(offset + 4, offset + 4 + length))
        for (let i = 0; i < length; i += 1) payload[i] ^= mask[i % 4]
        pending = pending.subarray(offset + 4 + length)
        if (opcode === 0x2 || opcode === 0x0) duplex.push(payload)
        else if (opcode === 0x9) socket.write(serverFrame(0xa, payload))
        else if (opcode === 0x8) socket.end()
      }
    })
  })
}

const listen = (server) =>
  new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server.address().port)))

/**
 * options:
 *   users    { name: password }                     (default: alice / bob)
 *   acl      'suback'  refuse out-of-namespace subscriptions, close the connection on a foreign publish
 *            'forward' accept everything, deliver only inside ah/<user>/   (Mosquitto, flespi)
 *            'none'    no separation at all
 *   quirks   noRetain        retained flag is dropped
 *            noWill          wills are never published
 *            willNotRetained wills are published without the retain flag
 *            willOnClean     the will is also published after a clean DISCONNECT
 *            maxPayload      a publish above this many bytes closes the connection
 *            refuseWildcard  subscriptions ending in /s/+ get a SUBACK failure
 *            maxIdleSessionMs  a session that has neither subscribed nor published is cut off this long after it
 *                            connects, whatever PINGREQs it sends (a gateway with a limit on quiet connections)
 */
async function startBroker(options = {}) {
  const users = options.users || { alice: 'pw-alice', bob: 'pw-bob' }
  const acl = options.acl || 'suback'
  const quirks = options.quirks || {}
  const aedes = await Aedes.createBroker({ maxClientsIdLength: 256 })

  aedes.preConnect = (client, packet, callback) => {
    client.sawWill = packet.will || null
    callback(null, true)
  }
  aedes.authenticate = (client, username, password, callback) => {
    const expected = users[username || '']
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
    if (client && quirks.noWill && packet === client.will) return callback(new Error('wills are off'))
    if (quirks.noRetain) packet.retain = false
    if (client && quirks.willNotRetained && packet === client.will) packet.retain = false
    if (client && quirks.maxPayload && packet.payload.length > quirks.maxPayload) return callback(new Error('too large'))
    if (acl === 'suback' && client && !(packet === client.will) && !inside(client.ahUser, packet.topic)) {
      return callback(new Error('publish refused'))
    }
    callback(null)
  }
  aedes.authorizeSubscribe = (client, subscription, callback) => {
    if (quirks.refuseWildcard && subscription.topic.endsWith('/s/+')) return callback(null, null)
    if (acl === 'suback' && !inside(client.ahUser, subscription.topic)) return callback(null, null)
    callback(null, subscription)
  }
  aedes.authorizeForward = (client, packet) => {
    if (acl === 'forward') {
      // Like a broker with per-user read and write rules that accepts every publish and then drops what
      // the publisher may not write, or the subscriber may not read.
      if (!inside(client.ahUser, packet.topic)) return null
      const writer = aedes.clients[packet.clientId]
      if (writer && !inside(writer.ahUser, packet.topic)) return null
    }
    return packet
  }
  if (quirks.willOnClean) {
    aedes.on('clientDisconnect', (client) => {
      const will = client.sawWill
      if (will) aedes.publish({ cmd: 'publish', topic: will.topic, payload: will.payload, qos: will.qos, retain: will.retain }, () => {})
    })
  }

  // A session that has subscribed or published is in use. Only the others count as idle for maxIdleSessionMs,
  // so a check that works through a session cannot be cut by how slowly the machine runs the checks before it.
  const markBusy = (_subscriptionsOrPacket, client) => {
    if (client && client.conn) client.conn.ahBusy = true
  }
  aedes.on('subscribe', markBusy)
  aedes.on('publish', markBusy)

  const sockets = new Set()
  const track = (stream) => {
    sockets.add(stream)
    stream.on('close', () => sockets.delete(stream))
    if (quirks.maxIdleSessionMs) setTimeout(() => !stream.ahBusy && stream.destroy(), quirks.maxIdleSessionMs).unref()
    aedes.handle(stream)
  }
  const tcp = net.createServer(track)
  const ws = webSocketServer(track)
  const port = await listen(tcp)
  const wsPort = await listen(ws)

  return {
    port,
    wsPort,
    users,
    async stop() {
      for (const stream of sockets) stream.destroy()
      tcp.close()
      ws.close()
      await new Promise((resolve) => aedes.close(resolve))
    },
  }
}

module.exports = { startBroker }
