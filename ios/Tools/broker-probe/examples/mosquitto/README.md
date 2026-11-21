# Mosquitto reference setup

A small, self-hosted [Mosquitto](https://mosquitto.org/) 2.x broker for the AskHuman iOS channel. It is
the "run it yourself" path; the hosted options are described in
[`docs/wiki/ios-broker.md`](../../../../../docs/wiki/ios-broker.md).

| File | What it is |
| --- | --- |
| [`mosquitto.conf`](mosquitto.conf) | Two TLS listeners (MQTT on 8883, MQTT over WebSocket on 8084), login required, retained messages kept across restarts. |
| [`acl`](acl) | One line: every account may use `ah/<its own user name>/` and nothing else. |

## What you need

- A machine the Mac and the phone can both reach (a small VPS is enough), with a public host name.
- A certificate for that name from a CA that browsers trust, for example Let's Encrypt. The apps do not
  accept a self-signed certificate.
- Mosquitto 2.x built with WebSocket support if you want the WSS fallback. The Debian, Ubuntu and
  Homebrew packages are.

## Setup

These are the usual Debian/Ubuntu paths. The files were tested on macOS (Homebrew), see
[what was tested](#what-was-tested); the Linux paths and service commands below are the standard ones
but were not run by us.

1. Install Mosquitto: `sudo apt install mosquitto`.
2. Use [`mosquitto.conf`](mosquitto.conf) as `/etc/mosquitto/mosquitto.conf` (keep a copy of the original).
   It is self-contained, so it replaces the package's default file rather than adding to it.
3. Create one account per person. The account name is the user name you type into the apps, and it can
   only contain letters and digits:

   ```sh
   sudo mosquitto_passwd -c /etc/mosquitto/passwd alice   # -c creates the file; leave it out for the next account
   ```

   The command asks for the password, so it does not end up in your shell history. Use a long random
   password.
4. Copy [`acl`](acl) to `/etc/mosquitto/acl` unchanged. The rule `pattern readwrite ah/%u/#` already gives
   every account its own `ah/<name>/` and nothing else.
5. Let the Mosquitto user own and read only its files; Mosquitto warns about files that others can read:

   ```sh
   sudo chown mosquitto: /etc/mosquitto/passwd /etc/mosquitto/acl
   sudo chmod 0600 /etc/mosquitto/passwd /etc/mosquitto/acl
   ```

6. Put the certificate and key where `mosquitto.conf` points (`/etc/mosquitto/certs/fullchain.pem` and
   `privkey.pem`), readable by the `mosquitto` user. With certbot, add a deploy hook that copies the
   renewed files there and runs `systemctl reload mosquitto`. A reload (SIGHUP) makes Mosquitto pick up
   a renewed certificate and key, new accounts in the password file and ACL changes, without a restart.
7. Open TCP 8883 (and 8084 for the fallback) in the firewall, then start the service:
   `sudo systemctl enable --now mosquitto`.

In the apps: host = your host name, port = 8883, WSS port = 8084, user name and password = the
account from step 3.

## Check it

Run the compatibility suite against the new broker from any machine (needs Node.js 22.12 or newer). Give
it two accounts so it can also prove that one account cannot reach another's topics:

```sh
cd ios/Tools/broker-probe && npm ci
AH_PROBE_PASSWORD='…' AH_PROBE_PASSWORD2='…' \
  node compat.cjs --host mqtt.example.com --user alice --user2 bob --port 8883 --ws-port 8084 --slow
```

Passwords are only read from the environment, never from the command line.

## What was tested

Mosquitto 2.1.2 from Homebrew on macOS (2026-11-21), with the two listeners of `mosquitto.conf` serving
a certificate from a throwaway local CA. `compat.cjs --slow` with two accounts passed every check on TLS
and on WSS:

- login, subscription, 15 QoS 1 messages between two sessions, retained messages, messages of 900,000
  bytes, idle keep-alive;
- the retained will is published when the connection drops, and not after a clean DISCONNECT;
- an account only receives messages inside its own `ah/<name>/` (live and retained), checked where it
  matters, at delivery;
- a connection that goes silent is declared lost after 1.5 times the keep-alive, so its will arrives after
  about 45 s for the apps' 30 s keep-alive (a measurement, not a pass/fail check).

Not tested: a proxy in front of Mosquitto (nginx, Caddy, a cloud load balancer) and Linux packaging. If
you terminate TLS in a proxy, run the suite through the proxy, and give the proxy read timeouts of more
than 90 s so that idle connections survive the 30 s keep-alive.
