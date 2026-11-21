# broker-probe

Checks for the MQTT broker behind the AskHuman iOS channel. They talk to a broker the way the Mac and
the phone do (MQTT 3.1.1 over TLS, and over WSS), so a pass means the channel will work on it. None of
them are part of any build; run them by hand against a broker before pointing the apps at it.

| Script | What it does | Needs |
| --- | --- | --- |
| `compat.cjs` | **The compatibility suite.** Everything the channel needs from a broker, ending in `COMPATIBLE`, `COMPATIBLE (with warnings)` or `NOT COMPATIBLE`. | a broker account (user name and password) |
| `acl-check.cjs` | Only the isolation part of the suite (an account must stay inside `ah/<its username>/`). Kept so old commands still work. | a broker account |
| `probe.cjs` | Reachability and latency of the TLS and WSS ports. | only the host name |

Requires Node 22.12 or newer. Run `npm install` once in this directory.

## compat.cjs

```bash
# The password comes from the environment, never from an argument, and is never printed.
AH_PROBE_PASSWORD='…' node compat.cjs --host broker.example.com --user alice

# With the WebSocket fallback, the slow measurements, and a Markdown table you can paste into a document
AH_PROBE_PASSWORD='…' node compat.cjs --host broker.example.com --user alice \
  --ws-port 443 --slow --label "Example broker" --markdown > result.md
```

Progress goes to stderr, reports (`--json`, `--markdown`) to stdout. Exit status: `0` compatible, `1` a
required check failed (or any warning with `--strict`), `2` wrong usage.

Options: `--port` (TLS, default 8883), `--ws-port` (WSS; `0` skips it, the default), `--ws-path`
(default `/mqtt`), `--password-env`, `--user2` and `--password2-env` (a second account, see below),
`--quick` (skip the 20 s idle check), `--slow`, `--keepalive-secs`, `--only isolation`, `--strict`,
`--label`, `--json`, `--markdown`, `--insecure-local` (plain TCP to a broker on this machine, for
tests; loopback only). `node compat.cjs --help` has the full text.

### What it checks

Each transport (TLS, and WSS if you give a port) is checked separately.

| Check | Level | Why the channel needs it |
| --- | --- | --- |
| connect | must | MQTT 3.1.1, clean session, user name and password, a certificate that ordinary clients trust |
| subscribe | must | QoS 1 granted for the app's topic filters, including the `+` presence wildcard |
| roundtrip | must | QoS 1 messages published by one session reach a second session of the same account (the Mac and the phone share one login) |
| retained | must | a retained message reaches a subscriber that arrives later (presence, pairing) |
| will | must | a retained QoS 1 will is published when a connection disappears, and replaces the retained "online" message (this is how a Mac that was killed turns "offline") |
| big | must | QoS 1 messages of 100 000, 332 000 and 900 000 bytes arrive intact (the largest envelope is 900 000 bytes) |
| keepalive | must | idle sessions that send PINGREQ are not cut off |

Also on each transport:

| Check | Level | What it tells you |
| --- | --- | --- |
| will-clean | should | an MQTT DISCONNECT does not trigger the will. MQTT says it must not. A broker that does anyway only adds a duplicate "offline" (both apps publish their own, retained, before they disconnect on purpose), so this is a warning. Seen on EMQX over WSS (not over TLS). |

Run once, on the first transport that connects:

| Check | Level | What it tells you |
| --- | --- | --- |
| isolation | should | whether a login can read or write outside `ah/<its username>/`. A message published outside it must never come back to the same session, in any of five forms (another user, a name that starts like ours, outside `ah/`, a wildcard across users). Judged at **delivery**: a refused subscription, a silently dropped message and a closed connection all count as isolated. |
| cross-user | should | with `--user2`: two accounts cannot read or write each other's topics, live or retained |

With `--slow` (about two minutes): `silent-will` (how long a connection that goes completely silent
takes to publish its will) and `ws-close-will` (what a WebSocket close without MQTT DISCONNECT does).
These are measurements, never verdicts.

**must** failures make the broker unfit; **should** failures are warnings (the channel works, but a
safeguard is missing or the broker differs in a harmless way); **info** is a measurement.

### Isolation is judged at delivery

Many brokers (Mosquitto, flespi, and EMQX with its default deny action) answer every subscription and
every publish with success, and then simply never deliver across accounts. That is a correct access
rule, and neither the SUBACK nor the PUBACK can show it. So the suite asks the only question that
matters: does a message outside the account's namespace ever come back?

If a broker only offers a coarse rule (for example "may use `ah/#`"), the result is a warning: any
holder of the login could read other users' topics. That does not stop the channel working for a single
user; the documentation says how to narrow the rule where the broker supports it.

### What it leaves behind

Everything lives under `ah/<username>/t/<run id>/…`, the subtree the apps' own connection test uses, so
any access rule written for the apps allows it. It never touches the `s/`, `d/` or `u/` topics a real
Mac or phone uses, and it clears the retained messages it creates. A full run moves about 3 MB per
transport (the three large messages dominate), which counts against a free monthly allowance.

## Self-tests

```bash
npm test
```

Runs the suite against real (Aedes) brokers on loopback: two compliant ones that keep accounts apart in
the two ways real brokers do (refusing, and accepting then dropping), one that does not separate
accounts at all, one that only earns a warning (a will after a clean disconnect), and one per way a broker
can be unfit (no retained messages, no will, a will that is not retained, a size limit, a refused
wildcard, cut-off idle sessions, a refused login). Each check has to give the right verdict in both
directions. CI runs this; it needs
no network and no credentials.

## Adding a broker to the documentation

Run the suite with `--ws-port` if the broker offers WSS, add `--markdown`, and attach the table and the
date to a pull request, with a link to the broker's own pages for the numbers you quote (free
allowance, ports, regions). A broker is described as "tested" only with a run like this behind it.
