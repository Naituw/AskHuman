'use strict'

// A small MQTT 3.1.1 client for the compatibility suite: it does exactly what the suite asks and nothing
// else (no reconnect, no retries), and it lets the suite end a connection in each of the ways a real client
// can go away (see transport.cjs).

const crypto = require('node:crypto')
const mqttPacket = require('mqtt-packet')
const { openPipe } = require('./transport.cjs')

const elapsedMs = (start) => Number(process.hrtime.bigint() - start) / 1e6

class MqttClient {
  /**
   * @param {object} endpoint `{ kind, host, port, path, insecureLocal, timeoutMs }`, see `openPipe`
   * @param {string} label    a short name used in the client id
   */
  constructor(endpoint, label) {
    this.endpoint = endpoint
    this.label = label
    this.publishes = []
    this.others = []
    this.waiters = []
    this.nextId = 1
    this.closed = false
    this.openMs = null
    this.connectMs = null
  }

  async open() {
    const { pipe, openMs } = await openPipe(this.endpoint)
    this.pipe = pipe
    this.openMs = openMs
    this.parser = mqttPacket.parser({ protocolVersion: 4 })
    this.parser.on('packet', (packet) => this.onPacket(packet))
    this.parser.on('error', () => this.pipe.drop())
    pipe.onBytes = (data) => this.parser.parse(data)
    pipe.onClose = () => {
      this.closed = true
      this.closedAt = process.hrtime.bigint()
      clearInterval(this.pingTimer)
      for (const waiter of this.waiters.splice(0)) {
        clearTimeout(waiter.timer)
        waiter.done(null)
      }
    }
    if (pipe.closed) pipe.onClose()
  }

  /**
   * CONNECT. Resolves with the CONNACK return code (0 = accepted), or null when the broker hung up or stayed
   * silent. Like a real client, the session sends PINGREQ at half the keepalive so the broker keeps it;
   * `autoPing: false` makes a session that stays silent on purpose.
   */
  async login({ user, pass, will, keepalive = 30, clientId, autoPing = true } = {}) {
    const started = process.hrtime.bigint()
    this.send({
      cmd: 'connect',
      protocolId: 'MQTT',
      protocolVersion: 4,
      clean: true,
      clientId: clientId || `ah-compat-${this.label}-${crypto.randomBytes(4).toString('hex')}`,
      keepalive,
      username: user,
      ...(pass === undefined || pass === null ? {} : { password: Buffer.from(pass) }),
      ...(will ? { will } : {}),
    })
    const connack = await this.expect((packet) => packet.cmd === 'connack', this.endpoint.timeoutMs || 10000)
    this.connectMs = elapsedMs(started)
    if (connack && connack.returnCode === 0 && autoPing) {
      this.pingTimer = setInterval(() => {
        if (!this.closed) this.send({ cmd: 'pingreq' })
      }, Math.max(1000, keepalive * 500))
      this.pingTimer.unref()
    }
    return connack ? connack.returnCode : null
  }

  send(packet) {
    this.pipe.write(mqttPacket.generate(packet, { protocolVersion: 4 }))
  }

  onPacket(packet) {
    if (packet.cmd === 'publish') {
      packet.receivedAt = process.hrtime.bigint()
      this.publishes.push(packet)
      this.notify()
      if (packet.qos === 1) this.send({ cmd: 'puback', messageId: packet.messageId })
      return
    }
    const index = this.waiters.findIndex((waiter) => !waiter.publish && waiter.match(packet))
    if (index >= 0) {
      const [waiter] = this.waiters.splice(index, 1)
      clearTimeout(waiter.timer)
      waiter.done(packet)
    } else {
      this.others.push(packet)
    }
  }

  notify() {
    for (const waiter of [...this.waiters]) {
      if (!waiter.publish) continue
      const index = this.publishes.findIndex(waiter.match)
      if (index < 0) continue
      this.waiters.splice(this.waiters.indexOf(waiter), 1)
      clearTimeout(waiter.timer)
      waiter.done(this.publishes.splice(index, 1)[0])
    }
  }

  /** Resolves with the first matching non-PUBLISH packet, or null when time runs out or the connection closes. */
  expect(match, ms = 8000) {
    const index = this.others.findIndex(match)
    if (index >= 0) return Promise.resolve(this.others.splice(index, 1)[0])
    if (this.closed) return Promise.resolve(null)
    return this.wait({ match, publish: false }, ms)
  }

  /** Resolves with the first received PUBLISH that matches, or null when time runs out or the connection closes. */
  expectPublish(match, ms = 8000) {
    const index = this.publishes.findIndex(match)
    if (index >= 0) return Promise.resolve(this.publishes.splice(index, 1)[0])
    if (this.closed) return Promise.resolve(null)
    return this.wait({ match, publish: true }, ms)
  }

  wait(waiter, ms) {
    return new Promise((done) => {
      const entry = { ...waiter, done }
      entry.timer = setTimeout(() => {
        const at = this.waiters.indexOf(entry)
        if (at >= 0) this.waiters.splice(at, 1)
        done(null)
      }, ms)
      this.waiters.push(entry)
    })
  }

  /** Every PUBLISH received so far that matches, without waiting. */
  takePublishes(match) {
    const taken = this.publishes.filter(match)
    this.publishes = this.publishes.filter((packet) => !match(packet))
    return taken
  }

  /** Resolves with the granted QoS (0–2, or 128 for a refusal), or null when no SUBACK came. */
  async subscribe(filter, qos = 1) {
    const messageId = this.nextId++
    this.send({ cmd: 'subscribe', messageId, subscriptions: [{ topic: filter, qos }] })
    const ack = await this.expect((packet) => packet.cmd === 'suback' && packet.messageId === messageId)
    return ack ? ack.granted[0] : null
  }

  /** QoS 1 publish. Resolves with the milliseconds until the PUBACK, or null when none came. */
  async publish(topic, payload, { retain = false, timeoutMs = 20000 } = {}) {
    const messageId = this.nextId++
    const started = process.hrtime.bigint()
    this.send({
      cmd: 'publish',
      topic,
      payload: Buffer.isBuffer(payload) ? payload : Buffer.from(payload),
      qos: 1,
      retain,
      dup: false,
      messageId,
    })
    const ack = await this.expect((packet) => packet.cmd === 'puback' && packet.messageId === messageId, timeoutMs)
    return ack ? elapsedMs(started) : null
  }

  /** PINGREQ; true when the PINGRESP came. */
  async ping(timeoutMs = 5000) {
    this.send({ cmd: 'pingreq' })
    return Boolean(await this.expect((packet) => packet.cmd === 'pingresp', timeoutMs))
  }

  /** The connection vanishes: no DISCONNECT, no WebSocket close frame (a killed process). */
  kill() {
    if (this.pipe) this.pipe.drop()
  }

  /** MQTT DISCONNECT, then close. */
  close() {
    if (!this.pipe) return
    this.send({ cmd: 'disconnect' })
    this.pipe.end()
  }
}

module.exports = { MqttClient, elapsedMs }
