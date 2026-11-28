---
name: relay-release
description: Publish a new version of the production AskHuman push relay (Tencent SCF function askhuman-relay), move its aliases, roll back, or change its reserved quota or per-instance concurrency, behind read-back gates that stop a release which would shrink the 16 concurrent slots. Use for any relay deployment or capacity-setting change.
---

# Relay release

A relay release is finished only when every gate below has passed. The gates exist because a version published with the default per-instance concurrency of 1 turns the 16 concurrent slots into 2. Real pushes are then refused with HTTP 432 (not executed, not billed, not retried by the daemon, so the push is lost), and the 432 alarm cannot see it: it needs five consecutive busy minutes. Numbers and evidence: `docs/specs/ios-open-access.md` §4.1–4.3 and §7. Read [`AGENTS.md`](../../../AGENTS.md) first.

## Established facts

- Function `askhuman-relay` (Web function, Node.js 20, 128 MB), region `ap-hongkong` (console `rid=5`), namespace `default`.
- Two aliases carry all traffic and must point at the **same single version, with no traffic split**: `live` (custom domain `https://askhuman-relay.maipo.me`, the app's default relay) and `$DEFAULT` (the original function URL, which the Owner's `~/.askhuman/config.json` keeps as `channels.ios.relayUrl`).
- Expected settings: reserved quota ("最大独占配额") **256 MB**, per-instance concurrency **8**, memory 128 MB, so 256 ÷ 128 × 8 = **16 slots**. The quota is function-level and survives publishing. The concurrency is version-level: it is copied from `$LATEST` when the version is published and cannot be edited afterwards, so a wrong one means publishing the next version.
- Versions 1 and 2 were published with concurrency 1. They stay as rollback targets with the caveat under "Rolling back".
- Publishing also freezes the environment variables of `$LATEST` (APNs key and so on). Changing one is a release too and goes through the same steps.
- If the expected numbers ever change (for example 512 MB × 8 = 32 slots), update `EXPECT` in `scripts/console-readback.js`, its tests, the facts here and spec §4.3 together.

## Rules

1. The console is reached through the Owner's own QR login, in the browser tab. **Do not ask for, create or use an API key.** Reading a function's configuration returns its environment variables, including the APNs private key and the handle key, so even a "read-only" key can read production secrets.
2. Never print, log, store or paste a function's `Environment` / `Variables` or a whole `GetFunction` response, and never call `GetFunctionAddress` (it returns a signed download link for the code package). `console-readback.js` returns a fixed set of fields; do not replace it with an ad-hoc dump.
3. A value that cannot be read is a failed check, not a skipped one. Do not go past a failed gate, and do not report the release done before all of them have passed.
4. Publishing, moving an alias and changing the quota or concurrency are done by hand in the console (the Owner, or you with the Owner's approval). The scripts only read and probe. Those write steps are not scripted and not covered by the tests.
5. Do not create a temporary function in this account for an experiment without telling the Owner first. Its 432s ring the real alarms, and deleting it leaves an alarm record that re-sends SMS and e-mail every hour (spec §4.3).
6. Never run the probe above 16 requests or against anything but the relay's own entrances.

## The two checks

Both live in `scripts/` next to this file. Their tests are `scripts/relay-release-*.node-test.mjs` in the repository's `scripts/` folder, like the other skill tests, so `npm test` runs them; alone: `node --test scripts/relay-release-*.node-test.mjs` from the repository root. After any change to a script, run them and a live `status` read-back.

### `console-readback.js`: configuration read-back

It runs inside the logged-in console page and replays the shape of the console's own `/cgi/capi` call with other read-only actions. Run it as follows.

1. Open `https://console.cloud.tencent.com/scf/list?rid=5&ns=default` with a full page load (this also clears anything an earlier session left in page memory). If a notice dialog appears, click 我知道了.
2. Evaluate the file in that tab with `Runtime.evaluate` (`awaitPromise` and `returnByValue` true; with the browser tool you can pass `"$file:<absolute path>/console-readback.js"` as `expression`). The first run installs a recording hook and answers `needTemplate`.
3. Click the refresh icon at the top right of the function table, so the page makes one console call for the hook to record.
4. Set the parameters in a separate evaluation, for example `window.__relayCheckParams = { phase: 'post', version: '4' }`, then evaluate the file again. Read `ok` and the `lines`; `ok` must be `true`.
5. When finished, navigate the tab again (drops the hook and the parameters).

If the console session has expired every line is `unreadable` and fails: ask the Owner to log in again. If the console's call shape has changed, the script fails closed instead of passing; fix it and its tests, and in the meantime read the same values by eye in the console and quote them.

| Phase | Use it | It requires |
| --- | --- | --- |
| `status` | before starting, and for a final look | `$LATEST` concurrency 8; quota 256; the version behind `live` (concurrency 8, Active, 128 MB); both aliases on that version with no split; slots ≥ 16; no `Http432` in the last 15 minutes |
| `pre` + `version` | after publishing version N, before touching an alias | `$LATEST` concurrency 8; quota 256; version N: concurrency 8, Active, 128 MB |
| `post` + `version` | after both aliases were moved to N | everything in `status`, with both aliases on N |
| `rollback` + `version` | after both aliases were moved back to V | like `post`, but V may have concurrency 1: slots ≥ 4 instead of 16, quota not pinned to 256, and the refusal count (which may still hold what caused the rollback) is information only |

### `probe.mjs`: what the entrances really do

```sh
node .agents/skills/relay-release/scripts/probe.mjs
```

No credentials. For each entrance it asks `GET /v1/health` (must be 200 with `ok` and `apns` true), then sends 12 simultaneous requests on separate connections: **all 12 must answer 200**. With 16 slots they all fit; a concurrency-1 version has 2 slots and refuses most of them with 432. The entrances come from `src-tauri/src/config.rs` (the default relay URL) and `~/.askhuman/config.json` (`channels.ios.relayUrl`, the old function URL); if the old URL cannot be found it fails instead of testing only one. Options: `--url <https URL>` (repeatable, replaces the discovery) and `--n <1..16>`. Exit 0 pass, 1 fail, 2 bad usage. Run it right after the aliases move: the new version has no warm instance, so the requests overlap on cold starts and a wrong concurrency cannot hide. The cost is negligible.

## Release

1. **Prepare.** Work from a clean tree at a definite commit. Run `cd relay && npm test` and `relay/scripts/build-zip.sh` (produces `relay/dist/askhuman-relay.zip`). The Owner must have the console logged in.
2. **Baseline.** Run the `status` read-back. Note which version both aliases point at (the rollback target) and its concurrency. If anything fails, production is already off the expected state: stop and tell the Owner before releasing on top of it.
3. **Check `$LATEST`.** Its concurrency must be 8 (the baseline shows it). If it is not, set it (`InstanceConcurrencyConfig`: `DynamicEnabled` FALSE, `MaxConcurrency` 8, `InstanceIsolationEnabled` FALSE, `Type` Request-Based; spec §4.3) and read it back. Change nothing else on it, and leave the environment variables alone.
4. **Upload and publish.** Upload the zip to `$LATEST` in the console (the file picker is native, so the Owner picks `relay/dist/askhuman-relay.zip`), then publish a new version N. Put the source commit's short hash in the version description.
5. **Gate 1: `pre`, version N.** It reads `$LATEST`, the quota and version N while the aliases still serve the old version, so a failure costs nothing. On failure do not move any alias. A wrong version N cannot be edited: fix `$LATEST` and publish N+1.
6. **Move `live` to N** (a single version, no weights), then `probe.mjs --url https://askhuman-relay.maipo.me` as a first look. If it fails, move `live` back at once.
7. **Move `$DEFAULT` to N.**
8. **Gate 2: `post`, version N.** Both aliases on N with no split, quota 256, slots ≥ 16, no refusals in the window. A `Http432` count above zero is not automatically this release's fault: the line lists the minutes (Beijing time) that had refusals, so compare them with the time you moved the aliases, and roll back if the refusals started after it.
9. **Gate 3: `probe.mjs`**, both entrances. If it fails, run it once more to rule out a network hiccup on your side; a second failure means roll back and investigate.
10. **Final read-back.** Do any documentation update you owe first, then run `status` once more: it must still pass, with no refusals, now including the probe's own traffic.
11. **Report** the new version, its commit, the rollback target and its concurrency, and quote the `lines` of the gates and the probe output. Say plainly that the write steps were done by hand.

## Rolling back

Trigger: a gate failed after an alias moved, or the Owner asks. Target: the previous version P from the baseline.

- If P has concurrency 8 (version 3 and every later one): move `live` and `$DEFAULT` back to P, then run `post` with P and `probe.mjs`.
- If P has concurrency 1 (versions 1 and 2): **first** set the quota to 512 MB (so there are 4 slots, not 2), **then** move the aliases, then run `rollback` with P and `probe.mjs --n 4`. Returning to a concurrency-8 version later: move the aliases first, **then** set the quota back to 256 MB, otherwise the in-between state has 2 slots.

## When a gate fails

| Failed line | Do |
| --- | --- |
| `$LATEST` concurrency is not 8 | Set it to 8, read it back, then publish a new version (an already published one keeps its value). |
| Version N concurrency is not 8 | Do not move aliases. Fix `$LATEST`, publish N+1, run `pre` again. |
| Quota is not 256 | Set the function's 最大独占配额 to 256 MB in the console (the function list shows the current value in its 最大独占配额 column) and read it back. Never set 128 MB: with concurrency 1 it once wedged the function in 432 until the quota was deleted (spec §4.3). |
| An alias is not on N, or splits traffic | Fix the alias so it points to N alone. |
| Slots below 16 | The slot count comes from the quota and from the concurrency of the versions behind the aliases, so the quota line, the version line or an alias line is failing too; fix that. |
| `Http432` above zero | See step 8. |
| Probe fails twice | Roll back, then investigate. |
| Anything `unreadable` | Treat as failed. Do not guess; see the note under the read-back steps. |
