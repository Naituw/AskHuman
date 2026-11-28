// Read-only read-back of the production relay function (Tencent SCF `askhuman-relay`).
//
// Run it inside a logged-in Tencent Cloud console tab; the exact steps are in ../SKILL.md. It
// replays the shape of the console's own /cgi/capi call with other read-only actions, so no API
// key is involved. Rules this file keeps:
//   * It never prints, stores or returns environment variables. Every response is cut down to a
//     small allowlist of fields before it is used, and responses are not kept on `window`.
//   * It fails closed: anything it cannot read, or reads in an unexpected shape, is a failed check.
//   * It only reads. Publishing, alias moves and quota changes stay manual.
//
// Parameters are read from `window.__relayCheckParams` (set it in a separate evaluation):
//   { phase: 'status' }                       current state; no version needed
//   { phase: 'pre',      version: '<N>' }     after publishing N, before any alias is moved
//   { phase: 'post',     version: '<N>' }     after both aliases were moved to N
//   { phase: 'rollback', version: '<V>' }     after both aliases were moved back to V
//
// Evaluate with awaitPromise + returnByValue. The result is a JSON string; `ok` must be true.
(async () => {
  const EXPECT = {
    functionName: 'askhuman-relay',
    namespace: 'default',
    region: 'ap-hongkong',
    regionId: 5,
    concurrency: 8, // per-instance concurrency of every version that serves traffic
    memory: 128,
    quotaMem: 256, // reserved quota in MB (function level)
    slotMem: 128, // one instance of this function
    aliases: ['live', '$DEFAULT'],
    minSlots: 16, // quota / slotMem * concurrency
    rollbackMinSlots: 4, // 512 MB x concurrency 1, the state before 2026-11-28
    http432WindowMin: 15,
  };
  const PHASES = ['status', 'pre', 'post', 'rollback'];

  const params = Object.assign({ phase: 'status', version: null }, window.__relayCheckParams || {});
  const phase = String(params.phase);
  let version = params.version == null ? null : String(params.version);

  // --- Request template: the console's own calls, recorded without their responses. ---
  const installHook = () => {
    const calls = [];
    const rawOpen = XMLHttpRequest.prototype.open;
    const rawSet = XMLHttpRequest.prototype.setRequestHeader;
    const rawSend = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (m, u) {
      this.__rbM = m;
      this.__rbU = String(u);
      this.__rbH = {};
      return rawOpen.apply(this, arguments);
    };
    XMLHttpRequest.prototype.setRequestHeader = function (k, v) {
      try {
        if (this.__rbH) this.__rbH[k] = v;
      } catch (e) {}
      return rawSet.apply(this, arguments);
    };
    XMLHttpRequest.prototype.send = function () {
      if (!this.__rbOwn && this.__rbM === 'POST' && /\/cgi\/capi/.test(this.__rbU || '')) {
        calls.push({ u: this.__rbU, h: this.__rbH || {} });
      }
      return rawSend.apply(this, arguments);
    };
    if (typeof window.fetch === 'function') {
      const rawFetch = window.fetch;
      window.fetch = function (input, init) {
        try {
          const u = typeof input === 'string' ? input : input && input.url;
          const method = (init && init.method) || (input && input.method) || 'GET';
          if (/\/cgi\/capi/.test(u || '') && String(method).toUpperCase() === 'POST') {
            const h = {};
            const src = (init && init.headers) || (input && input.headers) || {};
            if (typeof src.forEach === 'function' && !Array.isArray(src)) src.forEach((v, k) => (h[k] = v));
            else if (Array.isArray(src)) src.forEach(([k, v]) => (h[k] = v));
            else Object.assign(h, src);
            calls.push({ u, h });
          }
        } catch (e) {}
        return rawFetch.apply(this, arguments);
      };
    }
    return { calls };
  };
  const hook = window.__relayRb || (window.__relayRb = installHook());
  // Prefer the most recent function-service call; any other console call has the same shape.
  const tmpl = [...hook.calls].reverse().find((c) => /serviceType=scf/.test(c.u)) || hook.calls[hook.calls.length - 1];
  if (!tmpl) {
    return JSON.stringify({
      ok: false,
      needTemplate: true,
      message:
        'Recording hook installed. Make the console page send one request (click the refresh button on the function list), then run this script again.',
    });
  }

  const call = (cmd, data, opts) =>
    new Promise((resolve, reject) => {
      const o = opts || {};
      const serviceType = o.serviceType || 'scf';
      const url = tmpl.u
        .replace(/cmd=[^&]+/, 'cmd=' + cmd)
        .replace(/serviceType=[^&]+/, 'serviceType=' + serviceType)
        .replace(/([?&])t=\d+/, '$1t=' + Date.now());
      const body = JSON.stringify({
        regionId: EXPECT.regionId,
        serviceType,
        cmd,
        data: Object.assign({ Action: cmd, Version: o.version || '2018-04-16', Region: EXPECT.region }, data),
      });
      const x = new XMLHttpRequest();
      x.__rbOwn = true;
      x.open('POST', url);
      for (const [k, v] of Object.entries(tmpl.h || {})) {
        try {
          x.setRequestHeader(k, v);
        } catch (e) {}
      }
      x.onload = () => {
        let j = null;
        try {
          j = JSON.parse(x.responseText);
        } catch (e) {}
        const r = j && j.data && j.data.data && j.data.data.Response;
        if (r && !r.Error) resolve(r);
        else reject(new Error(r && r.Error ? r.Error.Code + ': ' + r.Error.Message : 'HTTP ' + x.status + ', unexpected response shape'));
      };
      x.onerror = () => reject(new Error('network error'));
      x.send(body);
    });
  const attempt = async (label, fn) => {
    try {
      return { v: await fn() };
    } catch (e) {
      return { err: label + ': ' + String((e && e.message) || e).slice(0, 160) };
    }
  };
  const fnArgs = { FunctionName: EXPECT.functionName, Namespace: EXPECT.namespace };

  // --- Allowlisted reads. ---
  const readVersion = async (q) => {
    const g = await call('GetFunction', Object.assign({ Qualifier: String(q), ShowCode: 'FALSE' }, fnArgs));
    const ic = g.InstanceConcurrencyConfig || {};
    return { qualifier: String(q), status: g.Status, memory: Number(g.MemorySize), concurrency: Number(ic.MaxConcurrency) };
  };
  const readAliases = async () => {
    const r = await call('ListAliases', fnArgs);
    const out = {};
    for (const a of r.Aliases || []) {
      const rc = a.RoutingConfig || {};
      const split = [rc.AdditionalVersionWeights, rc.AddtionVersionMatchs, rc.AdditionVersionMatches].some((l) => Array.isArray(l) && l.length > 0);
      out[a.Name] = { version: String(a.FunctionVersion), split };
    }
    return out;
  };
  const readQuota = async () => Number((await call('GetReservedConcurrencyConfig', fnArgs)).ReservedMem);
  const readHttp432 = async () => {
    const end = new Date();
    const start = new Date(end.getTime() - EXPECT.http432WindowMin * 60000);
    const iso = (d) => new Date(d.getTime() + 8 * 3600000).toISOString().slice(0, 19) + '+08:00';
    const r = await call(
      'GetMonitorData',
      {
        StartTime: iso(start),
        EndTime: iso(end),
        MetricName: 'Http432',
        Namespace: 'QCE/SCF_V2',
        Period: 60,
        Instances: [{ Dimensions: [{ Name: 'namespace', Value: EXPECT.namespace }, { Name: 'functionName', Value: EXPECT.functionName }] }],
      },
      { serviceType: 'monitor', version: '2018-07-24' },
    );
    const dp = (r.DataPoints || [])[0];
    const vals = dp && Array.isArray(dp.Values) ? dp.Values.map(Number) : [];
    const ts = dp && Array.isArray(dp.Timestamps) && dp.Timestamps.length === vals.length ? dp.Timestamps.map(Number) : null;
    // Beijing time (UTC+8), the clock the console and the Owner use; timestamps are seconds.
    const clock = (t) => new Date((t > 1e12 ? t : t * 1000) + 8 * 3600000).toISOString().slice(11, 16);
    const minutes = [];
    vals.forEach((v, i) => {
      if (v > 0) minutes.push((ts && Number.isFinite(ts[i]) ? clock(ts[i]) : 'point ' + (i + 1) + ' of ' + vals.length) + ' x' + v);
    });
    return { points: vals.length, total: vals.reduce((a, b) => a + b, 0), minutes };
  };

  // --- Checks. ---
  const checks = [];
  const add = (name, expect, actual, pass) => checks.push({ name, expect: String(expect), actual: String(actual), pass: pass === true });
  const num = (r, pick) => (r.err ? 'unreadable (' + r.err + ')' : pick(r.v));

  if (!PHASES.includes(phase)) add('phase is one of ' + PHASES.join('/'), 'valid', phase, false);

  const aliasesR = await attempt('aliases', readAliases);
  const quotaR = await attempt('quota', readQuota);
  const latestR = await attempt('$LATEST', () => readVersion('$LATEST'));
  if (PHASES.includes(phase) && phase === 'status' && !version && aliasesR.v && aliasesR.v.live) version = aliasesR.v.live.version;
  if (PHASES.includes(phase) && phase !== 'status' && !version) add('params.version given for phase ' + phase, 'a version number', 'missing', false);

  // Every version we need a concurrency reading for: the subject and whatever the aliases point at.
  const wanted = new Set();
  if (version) wanted.add(version);
  if (aliasesR.v && phase !== 'pre') for (const a of EXPECT.aliases) if (aliasesR.v[a]) wanted.add(aliasesR.v[a].version);
  const versions = {};
  for (const q of wanted) versions[q] = await attempt('version ' + q, () => readVersion(q));

  add('$LATEST per-instance concurrency (snapshotted by the next publish)', EXPECT.concurrency, num(latestR, (v) => v.concurrency), latestR.v && latestR.v.concurrency === EXPECT.concurrency);
  // Rolling back to a concurrency-1 version needs a different quota (512 MB), so a rollback is
  // judged by the slot count below instead of by this exact value.
  if (phase !== 'rollback') add('reserved quota (MB)', EXPECT.quotaMem, num(quotaR, (v) => v), quotaR.v === EXPECT.quotaMem);

  if (version && versions[version]) {
    const vr = versions[version];
    const expectConcurrency = phase === 'rollback' ? 'any, see slots' : EXPECT.concurrency;
    if (phase !== 'rollback') add('version ' + version + ' per-instance concurrency', expectConcurrency, num(vr, (v) => v.concurrency), vr.v && vr.v.concurrency === EXPECT.concurrency);
    add('version ' + version + ' status', 'Active', num(vr, (v) => v.status), vr.v && vr.v.status === 'Active');
    add('version ' + version + ' memory (MB)', EXPECT.memory, num(vr, (v) => v.memory), vr.v && vr.v.memory === EXPECT.memory);
  }

  if (phase !== 'pre' && PHASES.includes(phase)) {
    for (const a of EXPECT.aliases) {
      const ar = aliasesR.err ? null : aliasesR.v[a];
      const actual = aliasesR.err ? 'unreadable (' + aliasesR.err + ')' : ar ? 'version ' + ar.version + (ar.split ? ' (traffic split configured)' : '') : 'alias missing';
      add('alias ' + a + ' points at the version under test', 'version ' + version + ', no split', actual, !!ar && !!version && ar.version === version && !ar.split);
    }
    // Slots the live traffic really has: quota / instance size * the smallest concurrency behind an alias.
    const ks = EXPECT.aliases.map((a) => (aliasesR.v && aliasesR.v[a] && versions[aliasesR.v[a].version] && versions[aliasesR.v[a].version].v ? versions[aliasesR.v[a].version].v.concurrency : NaN));
    const minK = Math.min(...ks);
    const slots = Number.isFinite(minK) && typeof quotaR.v === 'number' ? (quotaR.v / EXPECT.slotMem) * minK : NaN;
    const need = phase === 'rollback' ? EXPECT.rollbackMinSlots : EXPECT.minSlots;
    add('concurrent slots behind the aliases (quota / ' + EXPECT.slotMem + ' MB x smallest concurrency)', '>= ' + need, Number.isFinite(slots) ? slots : 'unreadable', Number.isFinite(slots) && slots >= need);

    const mon = await attempt('Http432', readHttp432);
    const monText = mon.err
      ? 'unreadable (' + mon.err + ')'
      : mon.v.total + ' over ' + mon.v.points + ' data points' + (mon.v.minutes.length ? ' (Beijing time: ' + mon.v.minutes.join(', ') + ')' : '');
    if (phase === 'rollback') {
      // A rollback usually follows refusals, and the window may still hold them: the count is
      // information here, but it must still be readable.
      add('Http432 in the last ' + EXPECT.http432WindowMin + ' minutes (information in a rollback)', 'readable', monText, !!mon.v);
    } else {
      add('Http432 in the last ' + EXPECT.http432WindowMin + ' minutes', 0, monText, !!mon.v && mon.v.total === 0);
    }
  } else if (PHASES.includes(phase)) {
    add('aliases are readable (info; they must still point at the old version)', 'readable', aliasesR.err ? 'unreadable (' + aliasesR.err + ')' : JSON.stringify(aliasesR.v), !aliasesR.err);
  }

  const lines = checks.map((c) => (c.pass ? 'PASS' : 'FAIL') + '  ' + c.name + '  expected ' + c.expect + ', got ' + c.actual);
  const versionTable = {};
  for (const q of Object.keys(versions)) versionTable[q] = versions[q].v || versions[q].err;
  return JSON.stringify({
    ok: checks.length > 0 && checks.every((c) => c.pass),
    phase,
    version,
    readAt: new Date().toISOString(),
    lines,
    checks,
    aliases: aliasesR.v || aliasesR.err,
    versions: versionTable,
  });
})()
