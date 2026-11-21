'use strict'

// Byte pipes to an MQTT broker for the compatibility suite.
//
//   tls  MQTT over TLS (what the Mac and the phone try first)
//   wss  MQTT over a WebSocket over TLS, subprotocol `mqtt` (the fallback for networks that only let web traffic through)
//   tcp  plain TCP, loopback only (the suite's own offline tests)
//   ws   plain WebSocket, loopback only (the suite's own offline tests)
//
// The WebSocket client is written out here instead of using the built-in one, because the suite has to
// do what a built-in client never lets it do: drop the TCP connection with no close frame (a killed
// process), send a close frame with no MQTT DISCONNECT, and go silent without closing anything.

const net = require('node:net')
const tls = require('node:tls')
const crypto = require('node:crypto')

const LOOPBACK = new Set(['127.0.0.1', 'localhost', '::1'])

/** One RFC 6455 frame from client to server (clients must mask). */
function wsFrame(opcode, payload) {
  const length = payload.length
  let header
  if (length < 126) {
    header = Buffer.from([0x80 | opcode, 0x80 | length])
  } else if (length < 65536) {
    header = Buffer.alloc(4)
    header[0] = 0x80 | opcode
    header[1] = 0x80 | 126
    header.writeUInt16BE(length, 2)
  } else {
    header = Buffer.alloc(10)
    header[0] = 0x80 | opcode
    header[1] = 0x80 | 127
    header.writeBigUInt64BE(BigInt(length), 2)
  }
  const mask = crypto.randomBytes(4)
  const body = Buffer.alloc(length)
  for (let i = 0; i < length; i += 1) body[i] = payload[i] ^ mask[i % 4]
  return Buffer.concat([header, mask, body])
}

/** A connected pipe. `onBytes` and `onClose` are set by the owner right after `openPipe` returns. */
class Pipe {
  constructor(kind, socket) {
    this.kind = kind
    this.socket = socket
    this.closed = false
    this.onBytes = () => {}
    this.onClose = () => {}
    this.pending = Buffer.alloc(0)
    this.serverClosed = false
    socket.on('error', () => {})
    socket.on('close', () => {
      this.closed = true
      this.onClose()
    })
  }

  get isWebSocket() { return this.kind === 'wss' || this.kind === 'ws' }

  write(buffer) {
    if (this.closed) return
    this.socket.write(this.isWebSocket ? wsFrame(0x2, buffer) : buffer)
  }

  /** The TCP connection disappears with no MQTT DISCONNECT and no WebSocket close frame. */
  drop() { this.socket.destroy() }

  /**
   * A polite close (after an MQTT DISCONNECT): everything already written is flushed first, so the broker
   * always sees the DISCONNECT before the end of the stream. WebSocket pipes send a close frame as well.
   * A peer that never answers is cut off after two seconds.
   */
  end() {
    if (this.closed) return
    if (this.isWebSocket) this.wsClose()
    this.socket.end()
    setTimeout(() => this.socket.destroy(), 2000).unref()
  }

  /** A WebSocket close frame with no MQTT DISCONNECT in front of it. */
  wsClose() {
    if (!this.isWebSocket || this.closed) return
    this.socket.write(wsFrame(0x8, Buffer.from([0x03, 0xe8])))
  }

  /** Server-to-client frames: payload of binary frames goes to `onBytes`, pings are answered, close is honoured. */
  drain() {
    for (;;) {
      if (this.pending.length < 2) return
      const opcode = this.pending[0] & 0x0f
      let length = this.pending[1] & 0x7f
      let offset = 2
      if (length === 126) {
        if (this.pending.length < 4) return
        length = this.pending.readUInt16BE(2)
        offset = 4
      } else if (length === 127) {
        if (this.pending.length < 10) return
        length = Number(this.pending.readBigUInt64BE(2))
        offset = 10
      }
      if (this.pending.length < offset + length) return
      const payload = this.pending.subarray(offset, offset + length)
      this.pending = this.pending.subarray(offset + length)
      if (opcode === 0x2 || opcode === 0x0) this.onBytes(payload)
      else if (opcode === 0x9) this.socket.write(wsFrame(0xa, payload))
      else if (opcode === 0x8) {
        this.serverClosed = true
        try { this.socket.end() } catch { /* already gone */ }
      }
    }
  }
}

function connectSocket(options) {
  const { kind, host, port, insecureLocal } = options
  const secure = kind === 'tls' || kind === 'wss'
  if (!secure && !(insecureLocal && LOOPBACK.has(host))) {
    throw new Error('plain tcp / ws transports only work with --insecure-local on a loopback host')
  }
  return new Promise((resolve, reject) => {
    // An IP address is not a valid TLS server name; the certificate is then matched against the address itself.
    const socket = secure
      ? tls.connect({ host, port, ...(net.isIP(host) ? {} : { servername: host }) })
      : net.connect({ host, port })
    const timer = setTimeout(() => {
      socket.destroy()
      reject(new Error(`no connection within ${options.timeoutMs} ms`))
    }, options.timeoutMs)
    socket.once(secure ? 'secureConnect' : 'connect', () => {
      clearTimeout(timer)
      socket.removeAllListeners('error')
      resolve(socket)
    })
    socket.once('error', (error) => {
      clearTimeout(timer)
      reject(error)
    })
  })
}

/** The HTTP upgrade to a WebSocket with subprotocol `mqtt`. Resolves with the bytes that followed the response head. */
function upgrade(socket, host, path, timeoutMs) {
  const key = crypto.randomBytes(16).toString('base64')
  socket.write(
    `GET ${path} HTTP/1.1\r\nHost: ${host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n` +
      `Sec-WebSocket-Key: ${key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: mqtt\r\n\r\n`,
  )
  return new Promise((resolve, reject) => {
    let head = Buffer.alloc(0)
    const timer = setTimeout(() => {
      socket.destroy()
      fail(new Error(`no answer to the WebSocket upgrade within ${timeoutMs} ms`))
    }, timeoutMs)
    const fail = (error) => {
      clearTimeout(timer)
      socket.off('data', onData)
      reject(error)
    }
    const onData = (data) => {
      head = Buffer.concat([head, data])
      const end = head.indexOf('\r\n\r\n')
      if (end < 0) return
      clearTimeout(timer)
      socket.off('data', onData)
      socket.off('close', onClose)
      const status = head.subarray(0, head.indexOf('\r\n')).toString()
      if (!status.includes(' 101 ')) return reject(new Error(`WebSocket upgrade refused: ${status}`))
      resolve(head.subarray(end + 4))
    }
    const onClose = () => fail(new Error('the connection closed during the WebSocket upgrade'))
    socket.on('data', onData)
    socket.once('close', onClose)
  })
}

/**
 * Connects and, for WebSocket kinds, upgrades. `options`:
 * `{ kind, host, port, path = '/mqtt', insecureLocal = false, timeoutMs = 10000 }`.
 * Resolves with `{ pipe, openMs }` (time until bytes can be sent).
 */
async function openPipe(options) {
  const settings = { path: '/mqtt', insecureLocal: false, timeoutMs: 10000, ...options }
  const started = process.hrtime.bigint()
  const socket = await connectSocket(settings)
  const pipe = new Pipe(settings.kind, socket)
  if (pipe.isWebSocket) {
    const rest = await upgrade(socket, settings.host, settings.path, settings.timeoutMs)
    pipe.pending = Buffer.from(rest)
    socket.on('data', (data) => {
      pipe.pending = Buffer.concat([pipe.pending, data])
      pipe.drain()
    })
    pipe.drain()
  } else {
    socket.on('data', (data) => pipe.onBytes(data))
  }
  return { pipe, openMs: Number(process.hrtime.bigint() - started) / 1e6 }
}

module.exports = { openPipe, wsFrame, LOOPBACK }
