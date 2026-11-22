# Brokers for the iOS channel: requirements, options and verification

[简体中文](./ios-broker.md) | English

The AskHuman iOS channel relays messages between your Mac and your iPhone through an MQTT broker. You bring the broker; AskHuman does not host or resell one. Messages are end-to-end encrypted, so the broker only sees ciphertext and metadata such as "who is online" (see [iOS app channel setup](./ios-setup.en.md#how-it-works-and-who-can-see-what)).

This page lists the **requirements** first, then three **tested** paths side by side: EMQX Cloud Serverless, flespi and a self-hosted Mosquitto. Any MQTT broker that meets the requirements works; run the repository's [compatibility suite](#verify-a-broker-yourself) against it to find out.

The broker steps on the iOS App card (Settings → Channels) summarize the paths below.

## What the broker must do

| Requirement | Why |
|---|---|
| MQTT 3.1.1, clean session, QoS 1 | The Mac and the phone both use this combination |
| Retained messages and a **retained QoS 1 Last Will** | "Online / offline" is a retained message; when a Mac is killed or loses its network, the will turns it into offline |
| Messages of at least about 1 MB | The app's largest encrypted envelope is 900,000 bytes; a broker with a smaller packet limit drops messages or closes the connection |
| A fixed user name and password; the user name has only letters and digits | The user name is also the topic namespace `ah/<username>/`; the Mac and the phone share one login |
| Several simultaneous sessions for one user name | A Mac, a phone and more Macs online at once |
| A TLS certificate from a public CA, default port 8883 | The apps do not accept self-signed certificates |
| (Optional) WSS, path `/mqtt`, default port 8084 | A fallback for networks that block 8883; only tried when TLS cannot connect |
| Account isolation: an account can only read and write `ah/<its own username>/#` | Required when several people share a broker; for one person it works without |

## The three paths compared

Figures come from measurements and official pages checked on 2026-11-21. Prices and terms of third-party services change, so the official pages (linked in each section) win. Cells that were not tested say "not tested".

| | EMQX Cloud Serverless | flespi | Self-hosted Mosquitto |
|---|---|---|---|
| Server location | The region you pick (the author's instance is in Hangzhou) | Europe | Your own server |
| Ports | TLS 8883; WSS 8084 | TLS 8883; WSS 443 | Your choice; the certificate must come from a public CA |
| Credentials | User name and password created in the console | User name = token ID, password = the full token | `mosquitto_passwd` |
| Account isolation | ACL as described below | Each token's ACL limited to `ah/<ID>/#` | `pattern readwrite ah/%u/#` |
| Free allowance | 1 million connection minutes and 1 GB of traffic a month | Free plan: the official restrictions table limits rates and storage and has no monthly traffic cap; officially "for testing and development", limits cannot be extended | None (you pay for the server) |
| Beyond the free allowance | The deployment stops and must be started again by hand in the console; going on means topping up first | A token over its rate is suspended for 1 minute; when retained storage is full, new retained messages are not stored | n/a |
| Barrier | Going past the free allowance needs a prepaid balance, the minimum top-up may be ¥100 (the official docs do not state it) | Paid plans start at €130/month; a free account is deleted after 60 days of inactivity | A server with a public address, a domain and a certificate |
| Access from mainland China (author's network, single measurement) | Domestic instance: QoS 1 publish to acknowledgement about 136 ms over TLS, 35 ms over WSS | Cross-border: QoS 1 publish to acknowledgement about 315 ms, connect 0.5–1 s | Depends on your server |
| Mainland multi-point probe (single sample, Saturday 21:00, 329 probe points, 292 of them home broadband) | Response time median about 33–36 ms | 8883 and 443 about 96% reachable; median about 205–290 ms; about 3.5–4.3% time out; about 10–13% of home-broadband probes slower than 0.9 s | Depends on your server |
| Compatibility suite | All required checks pass on TLS and WSS; 1 warning (below) | All required checks pass on TLS and WSS, no warnings | All checks pass on TLS and WSS, no warnings |
| Known differences | Over WSS the broker still publishes the will after `DISCONNECT` (harmless, below); a silently lost connection publishes its will after about 45 s | A silently lost connection publishes its will only after about 90 s (3 times the keepalive) | A silently lost connection publishes its will after about 46 s; you maintain the certificate and any reverse proxy |

The "Access from mainland China" row was measured on the author's network in mainland China without a proxy and only describes that moment and that route; the "Mainland multi-point probe" row comes from the Chinaz multi-location port check and is also a single sample.

## Path 1: EMQX Cloud Serverless (hosted)

Compatibility suite (Cloud Serverless, Hangzhou, 2026-11-21): all required checks pass on TLS and WSS, and account isolation passes. The only warning is that over WSS the broker publishes the will after a clean disconnect (below).

### Setup

1. Create a **Serverless** deployment on [EMQX Cloud](https://cloud.emqx.com/) (China site) and **keep the spending limit at 0** (the default; only the free allowance is used). The international site is [cloud-intl.emqx.com](https://cloud-intl.emqx.com/); its prices are on its own site, and the figures here are from the China site.
2. In the deployment's "Overview", note the **connection address**. Ports: TLS `8883`, WSS `8084`.
3. **Authentication** ("Access Control → Authentication"): add a user name and password. Use **letters and digits only** for the user name.
4. **Authorization (ACL)** ("Access Control → Authorization"). This page defaults to **blacklist mode: anything not explicitly denied is allowed**, so rule 1 below is **required**, or any account can read and write other people's topics:
   1. On the "All users" tab add: topic `#`, action "Publish & Subscribe", **Deny** (the catch-all).
   2. On the "Username" tab add: the user name from the previous step, topic `ah/<username>/#`, action "Publish & Subscribe", **Allow**. EMQX checks the account's own rules first and the "All users" rules after, so only its own topics get through.
5. Enter host, ports, user name and password in AskHuman (see [iOS app channel setup](./ios-setup.en.md#1-connect-a-broker)), then [run the compatibility suite](#verify-a-broker-yourself) to confirm the isolation works.

### Free allowance and usage

- Free every month: **1 million connection minutes and 1 GB of traffic** (all public traffic into and out of the deployment; one message counts once when published and once when delivered), plus 1 million rule actions (AskHuman does not use them). Beyond that: ¥8 per million connection minutes and ¥1.5 per GB (official prices, [billing](https://docs.emqx.com/en/cloud/latest/price/pricing.html)).
- Where to look: the deployment's "Overview" page shows this month's connection minutes and traffic, counted hourly, so it lags about an hour; at 75% of the free allowance EMQX emails the account ([deployment FAQ](https://docs.emqx.com/zh/cloud/latest/faq/deploy.html), Chinese).
- AskHuman shows this month's traffic too (counted on this device: the Mac settings page and the phone's "Macs" page) and reminds you near each whole GB. Attachments are the big part: opening a picture of about 2 MB on the phone counts about 7 MB at the broker.

### Drawbacks

- **Going past the free allowance needs prepaid money.** With the spending limit at 0, the deployment stops as soon as either connection minutes or traffic run out, and the app says "the broker refused the connection, this month's free allowance may be used up". To keep going in the same month you raise the spending limit, and that needs a balance on the account. **The minimum top-up may be ¥100**; EMQX's official docs do not state a minimum, so go by what the console's top-up page says.
- **A stopped deployment does not come back by itself.** Neither the new allowance next month nor a higher limit starts it again; you start it by hand in the console.
- **Unused deployments are reclaimed.** With no active client for 30 days the system stops it; if it stays stopped for another 30 days it may be deleted, and a deleted deployment cannot be restored ([creating a Serverless deployment](https://docs.emqx.com/zh/cloud/latest/create/serverless.html), Chinese).
- Connection minutes are rarely the problem: a connection that is online all day is about 43,000 connection minutes a month, so a Mac plus a phone stays far below 1 million. Traffic is what to watch.

### The will after DISCONNECT over WSS

Over EMQX's WSS the compatibility suite reports one warning: when a client sends `DISCONNECT` and closes the WebSocket straight away, the broker still publishes the will (3 times out of 3; waiting 300 ms after `DISCONNECT` before closing, 0 out of 2; over TLS, 0 out of 6). MQTT says the will must not be published then. It makes no difference to AskHuman: before disconnecting on purpose, the Mac and the phone both publish their own retained "offline" first, so at worst there is a duplicate "offline" and the state is never wrong.

## Path 2: flespi (hosted, Europe)

Compatibility suite (Free plan, 2026-11-21): all required checks pass on TLS and WSS; with the narrow ACL below, account isolation and the two-account cross check pass too; no warnings.

### Setup

1. Register at [flespi.io](https://flespi.io/) and stay on the **Free** plan.
2. In the left menu open "Tokens", then the "+" at the bottom right to create a token:
   - **Choose type ACL.** Keep a single ACL entry: module `mqtt`, actions `publish` and `subscribe`, topic `ah/#` for now (you narrow it right after creating the token).
   - **Set a TTL.** flespi does not allow a token without a time limit. Enter the largest value the panel allows, **8760 hours (one year)**. The TTL counts from the last use, and an online MQTT session counts as use, so a Mac that stays online never lets it expire; if the token is unused for a whole year it is deleted and cannot be restored, and you create a new one and update both sides ([about tokens](https://flespi.com/kb/tokens-access-keys-to-flespi-platform)).
3. After creating it, every token in the list shows a **number** starting with `#` at its top left. **Open the token and change the ACL topic to `ah/<number>/#`** (the "Show JSON" button lets you edit the `acl` array directly: `[{"uri":"mqtt","actions":["publish","subscribe"],"topic":"ah/<number>/#"}]`) and save. flespi ACLs deny by default and only this one entry is allowed, which makes the isolation hold.
4. In AskHuman enter:

   | Field | Value |
   |---|---|
   | Broker host | `mqtt.flespi.io` |
   | TLS port | `8883` |
   | WSS port | `443` |
   | User name | The token **number** (digits only) |
   | Password | The **full token** (the full value is only available when the token is created, so copy it then; the list shows it masked afterwards) |

   flespi's own examples put the token in the user name, but it also accepts it as the password ([MQTT API page](https://flespi.com/mqtt-api)). AskHuman's user name is also the topic namespace, so it uses "number as user name, token as password": the author measured that this connects and that any other user name is refused.
5. [Run the compatibility suite](#verify-a-broker-yourself) to confirm.

> flespi enforces ACLs at **delivery**: subscribing and publishing both succeed, but messages across accounts are never delivered. The suite judges isolation that way, so "accepted, then dropped at delivery" is normal.

### Free allowance and usage

- The Free plan has no monthly traffic cap; the limits are rates and storage: 50 simultaneous sessions (the Mac and the phone take one each), 200 subscriptions per session, 20,000 messages per minute, 100 MiB of retained messages, 100 MiB of session storage, and REST/MQTT traffic of 50 MiB per minute (a token over it is suspended for 1 minute). flespi states that these limits **cannot be extended** ([restrictions](https://flespi.com/docs/flespi_en_restrictions.pdf), the version published 2026-08-12; [pricing](https://flespi.com/pricing)).
- Where to look: after signing in, the dashboard's "MQTT" card shows sessions, messages and retained storage, and the "Total traffic" card shows traffic for 1 hour / 24 hours / 7 days / 30 days / 1 year; the API shows them too.
- The monthly traffic AskHuman shows only counts this device's messages.

### Drawbacks

- **The Free plan is meant for "testing and development".** flespi may change the restrictions unilaterally (changes take effect at least 25 days after publication). The paid Commercial plan starts at €130/month and there is no cheaper tier ([pricing](https://flespi.com/pricing)).
- **A free account is deleted after 60 days of inactivity** (you can register again with the same email).
- **The servers are in Europe, so latency across the border is high.** On the author's network a QoS 1 publish to acknowledgement took about 315 ms, connecting 0.5–1 s, and a 900 kB message 1.6–8.5 s (several measurements). In the Chinaz multi-location probe about 3.5–4.3% of the probe points timed out.
- A connection that is **silently lost** (no packets and no close) publishes its will only after about 90 s, three times the keepalive; when the Mac process is killed and TCP drops, the will is published within 0.3–0.9 s.
- When a client closes the WebSocket over WSS without sending `DISCONNECT`, flespi does not publish the will. AskHuman always sends `DISCONNECT` first, so it is not affected.

## Path 3: self-hosted Mosquitto

The reference configuration and step-by-step instructions are in the repository at [`ios/Tools/broker-probe/examples/mosquitto/`](../../ios/Tools/broker-probe/examples/mosquitto/README.md): a `mosquitto.conf`, an ACL file with the single line `pattern readwrite ah/%u/#`, and the steps for installing, adding users, certificates and opening ports.

Key points:

- Account isolation is one rule: `pattern readwrite ah/%u/#` gives every user only their own namespace, and adding a user needs no rule change.
- The certificate must come from a public CA (for example Let's Encrypt); after renewing it, send `SIGHUP` to Mosquitto to reload it without a restart.
- The reference configuration opens both TLS 8883 and WSS 8084 (path `/mqtt`). If you do not need WSS, delete the second listener; keep the WSS port in the app at its default, because it is not used while TLS can connect.

Compatibility suite (Mosquitto 2.1.2, macOS, installed with Homebrew, a throwaway self-signed CA, 2026-11-21): every check passes on TLS and WSS, account isolation and the two-account cross check pass, no warnings; a silently lost connection publishes its will after about 45.6 s; closing the WebSocket without `DISCONNECT` publishes no will (no effect). **Not tested**: Linux distribution packages, and deployments behind a reverse proxy (nginx, Caddy and so on); behind a proxy, set its read timeout above 90 seconds.

## Verify a broker yourself

**Everyday check:** "Test connection" in the iOS channel card of Settings (on the command line, `AskHuman channel test ios`) connects to the broker, sends a test message to itself and waits for the echo, then publishes one non-retained message to a random topic outside its own namespace. If that message comes back within about 1.5 seconds, it warns that the account can read and write topics outside its own. Being refused, dropped or disconnected all count as isolated. The warning never blocks saving or pairing. It checks only this; the compatibility suite below checks everything.

The repository's compatibility suite works with any broker: it connects the way the app does (MQTT 3.1.1, over TLS and WSS), checks every requirement above and ends with `COMPATIBLE`, `COMPATIBLE (with warnings)` or `NOT COMPATIBLE`.

```bash
git clone https://github.com/Naituw/AskHuman && cd AskHuman/ios/Tools/broker-probe
npm ci                       # needs Node 22.12 or newer
# The password is read from the environment only: never an argument, never printed
AH_PROBE_PASSWORD='…' node compat.cjs --host <broker host> --user <username> --port 8883 --ws-port 8084
```

- Without `--ws-port`, WSS is not tested (for flespi use `443`).
- `--user2 <another user name>` (its password in the environment variable `AH_PROBE_PASSWORD2`) also proves that two accounts cannot read each other's topics.
- `--slow` adds about two minutes and measures how long a silently lost connection takes to publish its will.
- `--markdown` prints a table you can paste into a document; `--json` prints machine-readable results.

Results come in three levels: a failed **must** check means the broker cannot be used; a failed **should** check is a warning (the channel works, but a safeguard is missing or there is a harmless difference); **info** is a measurement. Exit status: 0 compatible, 1 a required check failed (or any warning with `--strict`), 2 wrong usage.

Every test topic is under `ah/<username>/t/<run id>/`, which your real Mac and phone never use, and the suite clears the retained messages it creates. A full run moves about 3 MB per transport, which counts against a free allowance. What each check means is in [`ios/Tools/broker-probe/README.md`](../../ios/Tools/broker-probe/README.md).

The repository also keeps `acl-check.cjs`, which runs only the isolation part; old commands keep working.

## Sharing a broker with friends

Everyone gets their own account, and each account may only use its own `ah/<username>/#`:

- **EMQX**: create a user name and password for each person, each with its own `ah/<username>/#` allow rule; leave the catch-all deny on "All users" alone.
- **flespi**: one token per person, each with its ACL limited to `ah/<that token's number>/#`. A free account allows 50 simultaneous sessions.
- **Mosquitto**: add users with `mosquitto_passwd`; the single `ah/%u/#` rule already covers everyone.

Then run the suite with `--user2` to confirm they cannot see each other.

## Submitting the result for another broker

Run the compatibility suite with `--markdown` (add `--ws-port` if the broker has WSS), and put the table, the date and the broker's version or plan in a pull request, with links to the official pages behind any figure you quote (free allowance, ports, regions). A broker is only called "tested" in the documentation after a run like this.
