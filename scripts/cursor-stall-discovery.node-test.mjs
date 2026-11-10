import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { test } from "node:test";

const discovery = vm.runInNewContext(fs.readFileSync(new URL("../src-tauri/src/cursor_desktop/stall_discovery.js", import.meta.url), "utf8"));

// A CDP surface with real JS functions and scopes, but no Cursor, processes, or model calls.
function inspector() {
  const objects = new Map(), closures = new Map();
  let serial = 0;
  const ref = value => {
    const objectId = String(++serial); objects.set(objectId, value);
    return {objectId, type:typeof value, description:typeof value === "function" ? value.toString() : "Closure"};
  };
  const send = async (method, params) => {
    const value = objects.get(params.objectId);
    if (method === "Runtime.callFunctionOn") {
      const mirror = [...closures.values()].some(scopes => scopes.includes(value));
      const result = vm.runInNewContext(`(${params.functionDeclaration})`).apply(mirror ? {} : value,
        (params.arguments ?? []).map(arg => arg.objectId ? objects.get(arg.objectId) : arg.value));
      return {result:params.returnByValue ? {value:result} : ref(result)};
    }
    if (method === "Runtime.getProperties") {
      return {result:Object.entries(value).map(([name, item]) => ({name,value:ref(item)})),
        internalProperties:closures.has(value) ? [{name:"[[Scopes]]",value:ref(closures.get(value))}] : []};
    }
    throw new Error(`Unexpected inspector command ${method}`);
  };
  return {ref,closures,api:discovery(send)};
}

function functions(suffix) {
  return vm.runInNewContext(`(() => {
    class Reader${suffix} { getStallDetectorTimeoutConfig() { return this.experiments.getDynamicConfig("nal_stall_detector_timeout_config"); } }
    class Host${suffix} { _noteTurnPrewarmTemplate(session, request) { return request.turnOptions.clone(); } }
    function retry${suffix}(options) { return options.hooks.classify(null, {stallRetryElapsedMs:0}); }
    function classify${suffix}(error, options, context) { return context.stallRetryElapsedMs >= window${suffix}; }
    function encode${suffix}(options) { return {stallFailTimeoutMs:options.stallDetectorConfig.failTimeoutMs}; }
    const table${suffix} = {stallDetectorConfig:{turn:{read(message) { return {failTimeoutMs:message.stallFailTimeoutMs}; }}}};
    const runnerExports = {}; const decoderExports = {};
    Object.defineProperties(runnerExports, {
      run${suffix}:{get:()=>retry${suffix}}, classify${suffix}:{get:()=>classify${suffix}},
      sideEffect:{get:()=>{throw Error("Must not read arbitrary getters");}},
    });
    Object.defineProperty(decoderExports,"table${suffix}",{get:()=>table${suffix}});
    return {scope:{Reader${suffix},Host${suffix},retry${suffix},classify${suffix},encode${suffix}},runnerExports,decoderExports,table:table${suffix}};
  })()`);
}

for (const suffix of ["Old_17", "$new92"]) test(`renderer locates renamed native bindings (${suffix})`, async () => {
  const cdp = inspector(), fixture = functions(suffix);
  const expression = await cdp.api.renderer({scopeChain:[{type:"module",object:cdp.ref(fixture.scope)}]});
  const context = vm.createContext({...fixture.scope,[`window${suffix}`]:240000});
  const binding = vm.runInContext(expression,context);
  assert.equal(binding.nativeRetryWindow,240000);
  assert.equal(typeof binding.reader.getStallDetectorTimeoutConfig,"function");
  const replacement = () => "wrapped";
  binding.setRunner(replacement);
  assert.equal(context[`retry${suffix}`],replacement);
  assert.equal(binding.runner(),replacement);
});

test("renderer rejects missing and ambiguous contracts without changing any binding", async () => {
  const cdp = inspector(), fixture = functions("One");
  const before = {...fixture.scope};
  fixture.scope.duplicate = fixture.scope.retryOne;
  const frame = {scopeChain:[{type:"module",object:cdp.ref(fixture.scope)}]};
  await assert.rejects(cdp.api.renderer(frame),/runner binding is ambiguous/);
  delete fixture.scope.duplicate;
  delete fixture.scope.ReaderOne;
  await assert.rejects(cdp.api.renderer(frame),/reader binding is unavailable/);
  assert.equal(fixture.scope.retryOne,before.retryOne);
});

function host(suffix, ids, loaded = true) {
  const cdp = inspector(), fixture = functions(suffix);
  function loader(id) { throw Error("Discovery must never load a module"); const entry = {exports:{}}; return factories[id].call(entry.exports); }
  loader.m = {
    [ids[0]]: function retryModule() { return {stallRetryElapsedMs:0}; },
    [ids[1]]: function decoderModule() { return {stallDetectorConfig:{turn:{}}}; },
  };
  const cache = loaded ? {[ids[0]]:{exports:fixture.runnerExports},[ids[1]]:{exports:fixture.decoderExports}} : {};
  function activate() { throw Error("Discovery must not activate an extension"); }
  cdp.closures.set(activate,[{[`loader${suffix}`]:loader}]);
  cdp.closures.set(loader,[{[`cache${suffix}`]:cache}]);
  const getter = Object.getOwnPropertyDescriptor(fixture.decoderExports,`table${suffix}`).get;
  cdp.closures.set(getter,[{[`table${suffix}`]:fixture.table}]);
  return {cdp,fixture,activate:cdp.ref(activate).objectId};
}

for (const [suffix, ids] of [["Old",["85606","36218"]],["New",["17","987654"]]]) {
  test(`Agent Host locates renamed loader, cache, module IDs and export bindings (${suffix})`, async () => {
    const {cdp,activate} = host(suffix,ids);
    const found = await cdp.api.host(activate);
    assert.equal(found.runner.id,ids[0]);
    assert.equal(found.runner.name,`retry${suffix}`);
    assert.equal(found.classifier.name,`classify${suffix}`);
    assert.equal(found.classifier.cutoff,`window${suffix}`);
    assert.equal(found.decoder.id,ids[1]);
    assert.equal(found.decoder.name,`table${suffix}`);
  });
}

test("Agent Host discovery leaves unloaded modules alone", async () => {
  const {cdp,activate} = host("Cold",["1","2"],false);
  await assert.rejects(cdp.api.host(activate),/has not loaded/);
});
