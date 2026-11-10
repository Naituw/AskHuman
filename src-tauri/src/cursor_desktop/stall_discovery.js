((send) => {
  "use strict";
  const identifier = /^[A-Za-z_$][\w$]*$/;
  // Inspect loaded code once per attachment. No version, minified symbol, module number,
  // factory execution, or live conversation is used to identify these bindings.
  function describe(value) {
    const text = fn => {
      try { return typeof fn === "function" ? Function.prototype.toString.call(fn) : ""; }
      catch { return ""; }
    };
    const own = (object, name) => object && Object.getOwnPropertyDescriptor(object, name)?.value;
    const source = text(value);
    const roles = [];
    if (/\bstallRetryElapsedMs\s*:/.test(source) && /\.classify\s*\(/.test(source)) roles.push("runner");
    const cutoff = /\bstallRetryElapsedMs\s*>=\s*([A-Za-z_$][\w$]*|\d+(?:\.\d+)?(?:e[+-]?\d+)?)/i.exec(source)?.[1];
    if (cutoff) roles.push("classifier");
    if (/\bstallFailTimeoutMs\s*:/.test(source) && source.includes(".stallDetectorConfig")) roles.push("encoder");
    const prototype = own(value, "prototype");
    if (text(own(prototype, "getStallDetectorTimeoutConfig")).includes("nal_stall_detector_timeout_config")) roles.push("reader");
    if (/\.turnOptions\.clone\s*\(/.test(text(own(prototype, "_noteTurnPrewarmTemplate")))) roles.push("host");
    if (text(own(own(own(value, "stallDetectorConfig"), "turn"), "read")).includes("stallFailTimeoutMs")) roles.push("decoder");
    return { roles, cutoff };
  }
  const call = async (objectId, fn, args = []) => {
    const result = await send("Runtime.callFunctionOn", {
      objectId, functionDeclaration: fn, arguments: args.map(value => ({ value })), returnByValue: true,
    });
    if (result.exceptionDetails) throw new Error("Cursor stall binding inspection failed");
    return result.result?.value;
  };
  const properties = objectId => send("Runtime.getProperties", { objectId, ownProperties: true });
  async function scopes(objectId) {
    const list = (await properties(objectId)).internalProperties?.find(item => item.name === "[[Scopes]]")?.value?.objectId;
    if (!list) throw new Error("Cursor module scopes are unavailable");
    return (await properties(list)).result.filter(item => /^\d+$/.test(item.name) && item.value?.objectId &&
      !/^Global\b/.test(item.value.description ?? "")).map(item => item.value.objectId).slice(0, 16);
  }
  const unique = (items, role) => {
    const found = items.filter(item => item.roles?.includes(role));
    if (found.length !== 1 || !identifier.test(found[0].name)) {
      throw new Error(`Cursor stall ${role} binding is ${found.length ? "ambiguous" : "unavailable"}`);
    }
    return found[0];
  };
  async function renderer(frame) {
    const candidates = [], names = new Set();
    for (const scope of frame.scopeChain.slice(0, 16)) {
      if (scope.type === "global" || scope.type === "local") continue;
      const found = await call(scope.object.objectId, `function() {
        const describe = ${describe};
        return Object.entries(Object.getOwnPropertyDescriptors(this)).flatMap(([name, entry]) => {
          if (typeof entry.value !== "function") return [];
          const info = describe(entry.value);
          return info.roles.length ? [{ name, ...info }] : [];
        });
      }`);
      for (const item of found) if (!names.has(item.name)) { names.add(item.name); candidates.push(item); }
    }
    const refs = Object.fromEntries(["reader", "host", "runner", "encoder", "classifier"].map(role => [role, unique(candidates, role)]));
    return `({reader:${refs.reader.name}.prototype,host:${refs.host.name}.prototype,
      classifier:${refs.classifier.name},nativeRetryWindow:${refs.classifier.cutoff},
      runner:()=>${refs.runner.name},setRunner:value=>{${refs.runner.name}=value;},
      encoder:()=>${refs.encoder.name},setEncoder:value=>{${refs.encoder.name}=value;}})`;
  }
  async function host(activate) {
    let loader;
    for (const scope of await scopes(activate)) {
      // Node's non-paused scope mirrors are only enumerable through CDP, unlike the paused
      // renderer scope. The loader's own objects can then be inspected without invoking it.
      const found = (await properties(scope)).result.filter(item => item.value?.type === "function" &&
        /\.call\([^)]*\.exports/.test(item.value.description ?? "") && /exports\s*:\s*\{\s*\}/.test(item.value.description ?? ""));
      if (found.length > 1) throw new Error("Cursor Agent Host module loader is ambiguous");
      if (found.length === 1) { loader = found[0].value.objectId; break; }
    }
    if (!loader) throw new Error("Cursor Agent Host module loader is unavailable");
    const modules = await call(loader, `function() {
      return Object.entries(this.m).flatMap(([id,factory]) => {
        const source = Function.prototype.toString.call(factory);
        const decoder = /stallDetectorConfig\\s*:\\s*\\{\\s*turn\\s*:/.test(source);
        return decoder || /stallRetryElapsedMs\\s*:/.test(source) ? [{id,decoder}] : [];
      });
    }`);
    if (modules.length < 1 || modules.length > 8) throw new Error("Cursor Agent Host stall modules are unavailable or ambiguous");
    const ids = modules.map(item => item.id);
    let cache;
    for (const scope of await scopes(loader)) {
      for (const item of (await properties(scope)).result) {
        if (item.value?.type !== "object" || !item.value.objectId) continue;
        const matches = await call(item.value.objectId, `function(ids) {
          return ids.every(id => {
            const module = Object.getOwnPropertyDescriptor(this, id)?.value;
            return !!(module && Object.getOwnPropertyDescriptor(module, "exports")?.value);
          });
        }`, [ids]);
        if (matches) {
          if (cache) throw new Error("Cursor Agent Host module cache is ambiguous");
          cache = item.value.objectId;
        }
      }
      if (cache) break;
    }
    if (!cache) throw new Error("Cursor has not loaded its Agent Host stall modules");
    const candidates = await call(cache, `function(ids) {
      const describe = ${describe};
      return ids.flatMap(id => Object.entries(Object.getOwnPropertyDescriptors(this[id].exports)).flatMap(([key, entry]) => {
        if (typeof entry.get !== "function") return [];
        const source = Function.prototype.toString.call(entry.get);
        const name = /^\\s*\\(\\s*\\)\\s*=>\\s*([A-Za-z_$][\\w$]*)\\s*$/.exec(source)?.[1] ??
          /^function\\s*[^ (]*\\(\\s*\\)\\s*\\{\\s*return\\s+([A-Za-z_$][\\w$]*)\\s*;?\\s*\\}$/.exec(source)?.[1];
        // Only read the bundler's plain binding getters, never arbitrary exports.
        if (!name) return [];
        const info = describe(Reflect.apply(entry.get, this[id].exports, []));
        return [{id, key, name, ...info}];
      }));
    }`, [ids]);
    const runner = unique(candidates, "runner"), classifier = unique(candidates, "classifier");
    if (runner.id !== classifier.id) throw new Error("Cursor Agent Host retry bindings use different modules");
    const tables = [];
    for (const module of modules.filter(item => item.decoder)) {
      const entry = candidates.find(item => item.id === module.id);
      if (!entry) continue;
      const getter = (await send("Runtime.callFunctionOn", {
        objectId: cache, functionDeclaration: "function(id,key) { return Object.getOwnPropertyDescriptor(this[id].exports,key).get; }",
        arguments: [{value:entry.id},{value:entry.key}],
      })).result?.objectId;
      if (!getter) continue;
      for (const scope of await scopes(getter)) {
        const values = (await properties(scope)).result.filter(item => item.value?.type === "object" && item.value.objectId);
        if (!values.length) continue;
        if (values.length > 1000) throw new Error("Cursor Agent Host decoder scope exceeds its inspection budget");
        const reply = await send("Runtime.callFunctionOn", {
          objectId: values[0].value.objectId, functionDeclaration: `function(...values) {
            const describe = ${describe};
            return values.map(value => describe(value).roles.includes("decoder"));
          }`, arguments: values.map(item => ({objectId:item.value.objectId})), returnByValue:true,
        });
        if (reply.exceptionDetails) throw new Error("Cursor Agent Host decoder inspection failed");
        const found = values.filter((_, index) => reply.result?.value?.[index]);
        tables.push(...found.map(item => ({id:entry.id,key:entry.key,name:item.name,roles:["decoder"]})));
        if (found.length) break;
      }
    }
    const decoder = unique(tables, "decoder");
    return { cache, runner, classifier, decoder };
  }
  return { renderer, host };
})
