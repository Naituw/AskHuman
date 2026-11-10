import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { test } from "node:test";

const source = name => fs.readFileSync(new URL(`../src-tauri/src/cursor_desktop/${name}.js`, import.meta.url), "utf8");
const policySource = source("stall_policy");
const load = name => source(name).replace("__ASKHUMAN_STALL_POLICY__", () => policySource);
const owner = "test-home", revision = "test-revision", token = "a1".repeat(16), other = "b2".repeat(16);
const hostInstaller = load("stall_host");
const policy = vm.runInNewContext(policySource);

// A stand-in for the local protobuf message. Unknown fields survive the native clone/send path.
const messageType = {
  typeName: "agent.v1.AgentHostTurnOptions",
  fields: { list: () => [] },
  runtime: { bin: {
    listUnknownFields: message => message.fields,
    discardUnknownFields: message => { message.fields = []; },
    onUnknownField: (message, no, wireType, data) => { message.fields.push({no, wireType, data}); },
  } },
};
class TurnOptions {
  constructor(advisory, fail, fields = []) {
    this.stallAdvisoryTimeoutMs = advisory;
    this.stallFailTimeoutMs = fail;
    this.fields = fields;
  }
  getType() { return messageType; }
  clone() { return new TurnOptions(this.stallAdvisoryTimeoutMs, this.stallFailTimeoutMs, this.fields.slice()); }
}

function nativeRunner(options) {
  const defaults = { stallRetryElapsedMs: 0, transportErrorRetries: 0 };
  return (error, context) => options.hooks.classify(error, {...defaults, ...context});
}

function nativeClassifier(error, options, context) {
  if (error === "stop" || context.transportErrorRetries >= (options.transportCap ?? 10)) return {action:"throw"};
  if (error === "stall" || context.abortOrigin === "stall_detector") {
    if (context.retriesEnabled === false || context.stallRetryElapsedMs>=18e4) return {action:"throw"};
    return {action:"retry",stall:true};
  }
  return {action:"retry"};
}
const classify = (error, context) => nativeClassifier(error, {}, context).action;

function host(hostToken = token) {
  const process = {pid: 123, getBuiltinModule: () => ({
    randomBytes: length => Buffer.from(hostToken, "hex").subarray(0, length),
  })};
  const context = vm.createContext({process});
  const installer = vm.runInContext(hostInstaller, context)(owner, revision);
  let runner = nativeRunner;
  const decoder = {
    read({stallAdvisoryTimeoutMs, stallFailTimeoutMs}) {
      return {advisoryTimeoutMs: stallAdvisoryTimeoutMs, failTimeoutMs: stallFailTimeoutMs};
    },
  };
  assert.equal(installer.attachRunner(() => runner, replacement => { runner = replacement; }, nativeClassifier, 180000), true);
  assert.equal(installer.attachDecoder(decoder).token, hostToken);
  return {
    decode: message => decoder.read(message),
    capture: config => runner({runOptions:{stallDetectorConfig:config},hooks:{classify}}),
  };
}

function window() {
  let nativeFail = 90000;
  const nativeHost = pid => ({pid, containsExtension: name => name === "anysphere.cursor-agent-host"});
  let manager = nativeHost(123);
  const environmentService = {isGlass:false};
  const experimentService = {getDynamicConfig: () => ({advisoryTimeoutMs:30000,failTimeoutMs:nativeFail})};
  class Agent {
    constructor() { this.environmentService = environmentService; this.experimentService = experimentService; }
    getStallDetectorTimeoutConfig() {
      if (this.environmentService.smokeTestNalStreamTimeoutMs !== undefined) {
        return {failTimeoutMs:this.environmentService.smokeTestNalStreamTimeoutMs};
      }
      return this.experimentService.getDynamicConfig("nal_stall_detector_timeout_config");
    }
  }
  class Provider {
    _noteTurnPrewarmTemplate(_session, request) { return request.turnOptions.clone (); }
  }
  function nativeEncode(options) {
    const raw = {stallFailTimeoutMs: options.stallDetectorConfig.failTimeoutMs};
    return new TurnOptions(options.stallDetectorConfig.advisoryTimeoutMs, raw.stallFailTimeoutMs);
  }
  const agent = new Agent(), provider = new Provider();
  const composer = {_environmentService:environmentService,_instantiationService:{_services:{_entries:new Map([
    ["experimentService",experimentService],
    ["extensionService",{_getExtensionHostManagers:()=>[manager]}],
  ])}}};
  const context = vm.createContext({Sme:Agent, fGo:Provider, E9_:nativeClassifier, hGo:nativeEncode, B9_:nativeRunner, composer});
  const code = load("stall_renderer").replace("__ASKHUMAN_OWNER__", JSON.stringify(owner))
    .replace("__ASKHUMAN_REVISION__", JSON.stringify(revision))
    .replace("__ASKHUMAN_STALL_BINDING__", `({reader:Sme.prototype,host:fGo.prototype,classifier:E9_,nativeRetryWindow:180000,
      runner:()=>B9_,setRunner:value=>{B9_=value;},encoder:()=>hGo,setEncoder:value=>{hGo=value;}})`);
  return {
    install: lease => vm.runInContext(`(function () {return ${code.replace("__ASKHUMAN_LEASE__", JSON.stringify(lease))};}).call(composer)`, context),
    state: () => context.__ASKHUMAN_CURSOR_STALL_V1__,
    read: () => agent.getStallDetectorTimeoutConfig(),
    encode: config => context.hGo({stallDetectorConfig:config}),
    warm: message => provider._noteTurnPrewarmTemplate("session",{turnOptions:message}),
    capture: config => context.B9_({runOptions:{stallDetectorConfig:config},hooks:{classify}}),
    native: ms => { nativeFail = ms; },
    restartHost: pid => { manager = nativeHost(pid); },
    smoke: ms => { environmentService.smokeTestNalStreamTimeoutMs = ms; },
  };
}

test("only paired snapshots extend both timers and survive disable while queued", () => {
  const renderer = window(), agentHost = host();
  assert.equal(renderer.install("lease").state,"pending");
  assert.equal(renderer.read().failTimeoutMs,90000);
  assert.equal(renderer.state().setHost(123,token,600000).state,"active");
  const active = renderer.read();
  assert.equal(active.failTimeoutMs,300000);
  assert.equal(active.advisoryTimeoutMs,30000);
  const activeRetry = renderer.capture(active);
  assert.equal(activeRetry("stall",{stallRetryElapsedMs:599999}),"retry");
  assert.equal(activeRetry("stall",{stallRetryElapsedMs:600000}),"throw");
  assert.equal(activeRetry("stop",{stallRetryElapsedMs:400000}),"throw");
  assert.equal(activeRetry("stall",{stallRetryElapsedMs:400000,transportErrorRetries:10}),"throw");
  assert.equal(renderer.state().restore(),true);
  assert.equal(renderer.read().failTimeoutMs,90000);
  const queued = renderer.encode(active);
  assert.equal(queued.stallFailTimeoutMs,90000);
  assert.equal(policy.tagged(queued),true);
  const warm = renderer.warm(queued);
  assert.equal(policy.tagged(warm),false);
  assert.equal(policy.tagged(queued),true);
  assert.equal(agentHost.decode(warm).failTimeoutMs,90000);
  const decoded = agentHost.decode(queued);
  assert.equal(policy.tagged(queued),false);
  assert.equal(decoded.failTimeoutMs,300000);
  const queuedRetry = agentHost.capture(decoded);
  assert.equal(queuedRetry("stall",{stallRetryElapsedMs:302000}),"retry");
  assert.equal(queuedRetry("stall",{stallRetryElapsedMs:600000}),"throw");
  assert.equal(agentHost.decode(renderer.encode(renderer.read())).failTimeoutMs,90000);
  const native300 = agentHost.decode(new TurnOptions(30000,300000));
  assert.equal(native300.failTimeoutMs,300000);
  assert.equal(agentHost.capture(native300)("stall",{stallRetryElapsedMs:180000}),"throw");
  assert.equal(renderer.install("next-lease").state,"pending");
  assert.equal(renderer.state().setHost(123,token,600000).state,"active");
});

test("a replaced Agent Host ignores the retired token and keeps native timeout and retries", () => {
  const renderer = window(), replacement = host(other);
  renderer.install("lease");
  renderer.state().setHost(123,token,600000);
  const inFlight = renderer.read();
  renderer.restartHost(124);
  assert.equal(renderer.read().failTimeoutMs,90000);
  assert.equal(renderer.state().setHost(123,token,600000).state,"pending");
  const message = renderer.encode(inFlight);
  messageType.runtime.bin.onUnknownField(message,98,0,new Uint8Array([1]));
  const decoded = replacement.decode(message);
  assert.equal(decoded.failTimeoutMs,90000);
  assert.equal(replacement.capture(decoded)("stall",{stallRetryElapsedMs:180000}),"throw");
  assert.deepEqual(message.fields.map(item=>item.no),[98]);
  assert.equal(renderer.state().setHost(124,other,600000).state,"active");
  assert.equal(replacement.decode(renderer.encode(renderer.read())).failTimeoutMs,300000);
});

test("disabled, longer and explicit native smoke-test timeouts are respected", () => {
  const renderer = window();
  renderer.install("lease");
  renderer.state().setHost(123,token,600000);
  for (const native of [0,300000,900000]) {
    renderer.native(native);
    assert.equal(renderer.state().activate().state,"notApplicable");
    assert.equal(renderer.read().failTimeoutMs,native);
    assert.equal(policy.tagged(renderer.encode(renderer.read())),false);
  }
  renderer.native(90000);
  renderer.smoke(125000);
  assert.equal(renderer.state().activate().state,"notApplicable");
  assert.equal(renderer.read().failTimeoutMs,125000);
});

test("native retry windows are measured, and a longer native window is never shortened", () => {
  for (const cutoff of [120000,240000,900000]) {
    const classifier = vm.runInNewContext(`(${nativeClassifier.toString().replace("18e4",String(cutoff))})`);
    policy.verifyClassifier(classifier,cutoff);
    const config = {}, tracked = new WeakSet([config]);
    const wrapped = policy.retryRunner(nativeRunner,tracked,cutoff);
    const capture = wrapped({runOptions:{stallDetectorConfig:config},hooks:{
      classify:(error,context)=>classifier(error,{},context),
    }});
    const effective = Math.max(cutoff,600000);
    assert.equal(capture("stall",{stallRetryElapsedMs:effective-1}).action,"retry");
    assert.equal(capture("stall",{stallRetryElapsedMs:effective}).action,"throw");
    assert.equal(capture("stop",{stallRetryElapsedMs:effective-1}).action,"throw");
    assert.equal(capture("stall",{stallRetryElapsedMs:0,transportErrorRetries:10}).action,"throw");
    assert.throws(()=>policy.verifyClassifier(classifier,cutoff+1),/preflight/);
  }
});

test("a mismatched Host retry policy never enables one half of the protection", () => {
  const renderer = window();
  renderer.install("lease");
  assert.equal(renderer.state().setHost(123,token,900000).state,"unavailable");
  assert.equal(renderer.read().failTimeoutMs,90000);
});

test("an interrupted Host attachment leaves native bindings discoverable for retry", () => {
  const context = vm.createContext({process:{pid:123,getBuiltinModule:()=>({
    randomBytes:()=>Buffer.from(token,"hex"),
  })}});
  const install = () => vm.runInContext(hostInstaller,context)(owner,revision);
  let runner = nativeRunner;
  const prepare = () => install().attachRunner(()=>runner,value=>{runner=value;},nativeClassifier,180000);
  prepare();
  assert.equal(runner,nativeRunner);
  assert.equal(install().status(),undefined);
  // A failed decoder preflight must also leave the runner unchanged.
  const decoder = {read(message) { return {failTimeoutMs:message.stallFailTimeoutMs}; }};
  assert.throws(()=>install().attachDecoder(decoder),/decoding preflight/);
  assert.equal(runner,nativeRunner);
  prepare();
  decoder.read = function ({stallAdvisoryTimeoutMs,stallFailTimeoutMs}) {
    return {advisoryTimeoutMs:stallAdvisoryTimeoutMs,failTimeoutMs:stallFailTimeoutMs};
  };
  assert.equal(install().attachDecoder(decoder).token,token);
  const attached = runner;
  prepare();
  assert.equal(install().attachDecoder(decoder).retryWindowMs,600000);
  assert.equal(runner,attached);
});
