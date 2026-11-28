// Tests for console-readback.js. The script is a browser snippet, so it runs here inside a vm
// context that stands in for a console tab: a window, and an XMLHttpRequest whose "server" is a
// table of canned answers. Nothing here contacts the console or reads a credential.
// Run: node --test scripts/relay-release-*.node-test.mjs (also part of `npm test`).
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const SRC = fs.readFileSync(fileURLToPath(new URL('../.agents/skills/relay-release/scripts/console-readback.js', import.meta.url)), 'utf8');
const SECRET = 'SECRET-APNS-KEY-MATERIAL';

// The production function in its intended state; tests override single fields.
function world(over = {}) {
  const w = {
    quota: 256,
    versions: {
      $LATEST: { k: 8, mem: 128, status: 'Active' },
      1: { k: 1, mem: 128, status: 'Active' },
      2: { k: 1, mem: 128, status: 'Active' },
      3: { k: 8, mem: 128, status: 'Active' },
    },
    aliases: { live: '3', $DEFAULT: '3' },
    split: {},
    http432: [0, 0, 0],
    fail: () => false,
    ...over,
  };
  return (cmd, data) => {
    const err = w.fail(cmd, data);
    if (err) return { Error: { Code: 'InternalError', Message: String(err) } };
    switch (cmd) {
      case 'GetFunction': {
        const v = w.versions[data.Qualifier];
        if (!v) return { Error: { Code: 'ResourceNotFound.FunctionVersion', Message: 'no such version' } };
        return {
          Status: v.status,
          MemorySize: v.mem,
          FunctionVersion: data.Qualifier,
          InstanceConcurrencyConfig: { DynamicEnabled: 'FALSE', MaxConcurrency: v.k, InstanceIsolationEnabled: 'FALSE', Type: 'Request-Based' },
          // The real response carries the environment; the script must never pass it on.
          Environment: { Variables: [{ Key: 'APNS_KEY_P8', Value: SECRET }] },
        };
      }
      case 'ListAliases':
        return {
          Aliases: Object.entries(w.aliases).map(([Name, FunctionVersion]) => ({
            Name,
            FunctionVersion,
            RoutingConfig: { AdditionalVersionWeights: w.split[Name] || [], AddtionVersionMatchs: [], AdditionVersionMatches: [] },
          })),
        };
      case 'GetReservedConcurrencyConfig':
        return { ReservedMem: w.quota };
      case 'GetMonitorData':
        return { DataPoints: [{ Values: w.http432, ...(w.http432At ? { Timestamps: w.http432At } : {}) }] };
      default:
        return {};
    }
  };
}

// A console tab. `serve` answers the capi calls; the page itself makes one call (the template).
function tab(serve) {
  const sent = [];
  class XMLHttpRequest {
    open(m, u) {
      this.m = m;
      this.u = u;
    }
    setRequestHeader() {}
    send(body) {
      const req = JSON.parse(body || '{}');
      sent.push({ cmd: req.cmd });
      queueMicrotask(() => {
        this.status = 200;
        this.responseText = JSON.stringify({ data: { data: { Response: req.cmd ? serve(req.cmd, req.data || {}) : {} } } });
        if (this.onload) this.onload();
      });
    }
  }
  const ctx = vm.createContext({ XMLHttpRequest, queueMicrotask });
  ctx.window = ctx;
  const pageCall = () =>
    vm.runInContext(
      `const x = new XMLHttpRequest();
       x.open('POST', '/cgi/capi?cmd=ListFunctions&action=delegate&serviceType=scf&secure=1&version=3&t=1&uin=o1&csrfCode=1');
       x.setRequestHeader('Content-Type', 'application/json');
       x.send('{}');`,
      ctx,
    );
  const run = async (params) => {
    ctx.window.__relayCheckParams = params;
    return JSON.parse(await vm.runInContext(SRC, ctx));
  };
  return { run, pageCall, sent };
}

const ready = (over, params) => async () => {
  const t = tab(world(over));
  const first = await t.run(params);
  assert.equal(first.needTemplate, true, 'the first run only installs the hook');
  t.pageCall();
  return t.run(params);
};
const failing = (r) => r.checks.filter((c) => !c.pass).map((c) => c.name);

test('without a recorded console call it asks for one and reads nothing', async () => {
  const t = tab(world());
  const r = await t.run({ phase: 'status' });
  assert.deepEqual([r.ok, r.needTemplate], [false, true]);
  assert.equal(t.sent.length, 0);
});

test('status: the intended production state passes', async () => {
  const r = await ready({}, { phase: 'status' })();
  assert.equal(r.ok, true, r.lines.join('\n'));
  assert.equal(r.version, '3');
});

test('pre: a new version with concurrency 8 passes while the aliases still point at the old one', async () => {
  const over = { versions: { $LATEST: { k: 8, mem: 128, status: 'Active' }, 3: { k: 8, mem: 128, status: 'Active' }, 4: { k: 8, mem: 128, status: 'Active' } } };
  const r = await ready(over, { phase: 'pre', version: '4' })();
  assert.equal(r.ok, true, r.lines.join('\n'));
});

test('pre: a new version published with concurrency 1 is stopped', async () => {
  const r = await ready({ versions: { $LATEST: { k: 8, mem: 128, status: 'Active' }, 3: { k: 8, mem: 128, status: 'Active' }, 4: { k: 1, mem: 128, status: 'Active' } } }, { phase: 'pre', version: '4' })();
  assert.equal(r.ok, false);
  assert.deepEqual(failing(r), ['version 4 per-instance concurrency']);
});

test('pre: $LATEST left at concurrency 1 is stopped even if the new version looks right', async () => {
  const r = await ready({ versions: { $LATEST: { k: 1, mem: 128, status: 'Active' }, 4: { k: 8, mem: 128, status: 'Active' } } }, { phase: 'pre', version: '4' })();
  assert.equal(r.ok, false);
  assert.match(failing(r)[0], /^\$LATEST per-instance concurrency/);
});

test('post: both aliases on the new version passes', async () => {
  const over = { versions: { $LATEST: { k: 8, mem: 128, status: 'Active' }, 4: { k: 8, mem: 128, status: 'Active' } }, aliases: { live: '4', $DEFAULT: '4' } };
  const r = await ready(over, { phase: 'post', version: '4' })();
  assert.equal(r.ok, true, r.lines.join('\n'));
});

test('post: an alias left on the old version is stopped', async () => {
  const over = { versions: { $LATEST: { k: 8, mem: 128, status: 'Active' }, 3: { k: 8, mem: 128, status: 'Active' }, 4: { k: 8, mem: 128, status: 'Active' } }, aliases: { live: '4', $DEFAULT: '3' } };
  const r = await ready(over, { phase: 'post', version: '4' })();
  assert.equal(r.ok, false);
  assert.deepEqual(failing(r), ['alias $DEFAULT points at the version under test']);
});

test('post: an alias that splits traffic is stopped', async () => {
  const split = [{ Version: '3', Weight: 0.5 }];
  const r = await ready({ split: { live: split } }, { phase: 'post', version: '3' })();
  assert.equal(r.ok, false);
  assert.deepEqual(failing(r), ['alias live points at the version under test']);
});

test('post: the old 16-slot-to-2 mistake is caught by the quota, the version and the slot count', async () => {
  const over = { versions: { $LATEST: { k: 8, mem: 128, status: 'Active' }, 4: { k: 1, mem: 128, status: 'Active' } }, aliases: { live: '4', $DEFAULT: '4' } };
  const r = await ready(over, { phase: 'post', version: '4' })();
  assert.equal(r.ok, false);
  assert.ok(failing(r).includes('version 4 per-instance concurrency'));
  assert.ok(failing(r).some((n) => n.startsWith('concurrent slots')));
  assert.equal(r.checks.find((c) => c.name.startsWith('concurrent slots')).actual, '2');
});

test('post: a changed quota is stopped, and so is a quota left at 512 with concurrency 8', async () => {
  for (const quota of [128, 512, 0]) {
    const r = await ready({ quota }, { phase: 'post', version: '3' })();
    assert.equal(r.ok, false, 'quota ' + quota);
    assert.ok(failing(r).includes('reserved quota (MB)'));
  }
});

test('post: refusals in the last minutes are stopped', async () => {
  const r = await ready({ http432: [0, 3, 0] }, { phase: 'post', version: '3' })();
  assert.equal(r.ok, false);
  assert.match(failing(r)[0], /^Http432/);
});

test('post: the refusals are listed with the minute they happened in (Beijing time)', async () => {
  const at = [1795000000, 1795000060, 1795000120];
  const clock = (t) => new Date(t * 1000 + 8 * 3600000).toISOString().slice(11, 16);
  const r = await ready({ http432: [0, 3, 0], http432At: at }, { phase: 'post', version: '3' })();
  assert.equal(r.ok, false);
  const line = r.lines.find((l) => l.includes('Http432'));
  assert.ok(line.includes(`${clock(at[1])} x3`), line);
  assert.ok(!line.includes(clock(at[0]) + ' x'), line);
  // Without timestamps the point's position is given instead of a clock time.
  const plain = await ready({ http432: [0, 3, 0] }, { phase: 'post', version: '3' })();
  assert.match(plain.lines.find((l) => l.includes('Http432')), /point 2 of 3 x3/);
});

test('rollback: version 1 with quota 512 passes (4 slots), with quota 256 it is stopped (2 slots)', async () => {
  const back = { aliases: { live: '1', $DEFAULT: '1' } };
  const good = await ready({ ...back, quota: 512 }, { phase: 'rollback', version: '1' })();
  assert.equal(good.ok, true, good.lines.join('\n'));
  const bad = await ready({ ...back, quota: 256 }, { phase: 'rollback', version: '1' })();
  assert.equal(bad.ok, false);
  assert.deepEqual(failing(bad), ['concurrent slots behind the aliases (quota / 128 MB x smallest concurrency)']);
  assert.equal(bad.checks.find((c) => c.name.startsWith('concurrent slots')).actual, '2');
});

test('rollback: refusals that caused the rollback are shown but do not fail it', async () => {
  const r = await ready({ aliases: { live: '1', $DEFAULT: '1' }, quota: 512, http432: [0, 5, 2] }, { phase: 'rollback', version: '1' })();
  assert.equal(r.ok, true, r.lines.join('\n'));
  assert.match(r.lines.find((l) => l.includes('Http432')), /got 7 over 3 data points/);
});

test('rollback: back to a concurrency-8 version keeps working with the normal quota', async () => {
  const r = await ready({}, { phase: 'rollback', version: '3' })();
  assert.equal(r.ok, true, r.lines.join('\n'));
});

test('fail closed: anything unreadable is a failed check, never a pass', async () => {
  for (const cmd of ['GetFunction', 'ListAliases', 'GetReservedConcurrencyConfig', 'GetMonitorData']) {
    const r = await ready({ fail: (c) => c === cmd && 'boom' }, { phase: 'post', version: '3' })();
    assert.equal(r.ok, false, cmd);
    assert.ok(r.checks.some((c) => !c.pass && /unreadable/.test(c.actual)), cmd);
  }
  // The refusal count is information in a rollback, but an unreadable one still fails.
  const back = { aliases: { live: '1', $DEFAULT: '1' }, quota: 512, fail: (c) => c === 'GetMonitorData' && 'boom' };
  assert.equal((await ready(back, { phase: 'rollback', version: '1' })()).ok, false);
});

test('bad parameters: unknown phase and missing version are failures', async () => {
  const a = await ready({}, { phase: 'nope' })();
  assert.equal(a.ok, false);
  const b = await ready({}, { phase: 'post' })();
  assert.equal(b.ok, false);
  assert.ok(failing(b).some((n) => n.startsWith('params.version')));
});

test('the environment of the function never leaves the script', async () => {
  const t = tab(world());
  await t.run({ phase: 'post', version: '3' });
  t.pageCall();
  const raw = JSON.stringify(await t.run({ phase: 'post', version: '3' }));
  assert.ok(!raw.includes(SECRET));
  assert.ok(!raw.includes('APNS_KEY_P8'));
});
