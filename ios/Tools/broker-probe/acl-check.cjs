#!/usr/bin/env node
'use strict'

// Access-rule check for the MQTT broker behind the iOS channel.
//
// This used to judge a broker by what it answered (SUBACK / PUBACK codes). That is the wrong question:
// Mosquitto, flespi and many others accept the subscription and the publish and then simply never
// deliver across accounts, which is a perfectly good access rule that those answers cannot show.
// The check now lives in compat.cjs and asks the only question that matters: does a message outside
// ah/<own username>/ ever come back? This file keeps the old command line working.
//
//   AH_PROBE_PASSWORD=… node acl-check.cjs --host <host> --user <username> [--port 8883] [--insecure-local]
//
// Same as:  node compat.cjs --only isolation --strict --host <host> --user <username> ...
// For everything else a broker has to do, run compat.cjs itself (see README.md).
//
// --other and --protocol are accepted and ignored: the other account is a random name now (it does not
// need to exist) and the channel speaks MQTT 3.1.1 only.

const args = process.argv.slice(2)
const forwarded = []
for (let i = 0; i < args.length; i += 1) {
  if (args[i] === '--other' || args[i] === '--protocol') {
    process.stderr.write(`note: ${args[i]} is no longer needed and is ignored\n`)
    i += 1
  } else if (args[i] === '--timeout-ms') {
    i += 1
  } else {
    forwarded.push(args[i])
  }
}
process.argv = [process.argv[0], require.resolve('./compat.cjs'), '--only', 'isolation', '--strict', ...forwarded]
require('./compat.cjs')
