import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {mkdtempSync, writeFileSync, readFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
const require = createRequire(import.meta.url);
const {attach} = require('../src-tauri/src/cursor_cli/preload.cjs');
const id = './src/shared/cli-background-work-registry.ts';
function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'ah-cli-unit-'));
  const options = {index:join(root,'index.js'), policy:join(root,'policy.json'), status:join(root,'process.json'), revision:'test'};
  writeFileSync(options.policy, JSON.stringify({enabled:true}));
  const originalLoad = () => ({});
  const module = {_load:originalLoad, _resolveFilename:()=>join(root,'2320.index.js')};
  const controller = attach(options, {module});
  t.after(()=>{controller.close();rmSync(root,{recursive:true,force:true});});
  class Core {
    constructor() { this.pending=[]; this.inflight=[]; this.results=new Map(); this.awaiting=new Set(); this.acked=[]; }
    pull() { for (const item of this.pending) item.payload ??= {kind:item.kind || 'shell'};const batch=this.pending.filter(x=>!this.awaiting.has(x.id));this.pending=this.pending.filter(x=>this.awaiting.has(x.id));this.inflight.push(...batch);return batch; }
    suppress(_, id) { this.pending=this.pending.filter(x=>x.id!==id); }
  }
  class Registry extends Core {
    pull() { return super.pull(); }
    forgetInFlight(ids) { this.inflight=this.inflight.filter(x=>!ids.includes(x.id)); }
    ack(ids) { this.acked.push(...ids);this.forgetInFlight(ids); }
  }
  const originalPull=Registry.prototype.pull;
  const chunk={modules:{[id]:(_module,exports)=>{exports.t=Registry;}}};
  controller.wrapChunk(chunk);
  const exports={}; chunk.modules[id]({},exports,{});
  return {controller,options,Registry,originalPull,module,originalLoad,setPolicy(value){writeFileSync(options.policy,JSON.stringify({enabled:value}));}};
}
test('consumes native notifications with ACK and preserves actual task results',t=>{
  const {Registry,controller}=fixture(t);const registry=new Registry();
  for(const kind of ['shell','subagent']) for(const outcome of ['success','error','aborted']) {
    const id=kind+outcome;registry.pending.push({id,kind,outcome});registry.results.set(id,{outcome,output:'actual output'});
  }
  assert.deepEqual(registry.pull('chat'),[]);assert.equal(registry.acked.length,6);assert.equal(registry.inflight.length,0);assert.equal(registry.results.size,6);assert.equal(controller.state.suppressed,6);
});
test('pending Await and completed results remain available without a new notification turn',t=>{
  const {Registry}=fixture(t);const registry=new Registry();
  registry.awaiting.add('await');registry.pending.push({id:'await'},{id:'idle'});registry.results.set('await',{exitCode:0,output:'await result'});
  assert.deepEqual(registry.pull('chat'),[]);assert.deepEqual(registry.acked,['idle']);
  registry.suppress('chat','await');registry.awaiting.delete('await');
  assert.deepEqual(registry.pull('chat'),[]);assert.equal(registry.results.get('await').output,'await result');
});
test('disable is checked at pull and re-enable restores the guard',t=>{
  const f=fixture(t);const registry=new f.Registry();f.setPolicy(false);registry.pending.push({id:'native'});
  assert.deepEqual(registry.pull('chat'),[{id:'native',payload:{kind:'shell'}}]);f.controller.refresh();assert.equal(f.Registry.prototype.pull,f.originalPull);
  f.setPolicy(true);f.controller.refresh();registry.pending.push({id:'managed'});assert.deepEqual(registry.pull('chat'),[]);assert.equal(f.controller.state.active,true);
});
test('restore preserves a later foreign prototype wrapper and reports incomplete coverage',t=>{
  const f=fixture(t);const foreign=()=>['foreign'];f.Registry.prototype.pull=foreign;f.controller.refresh();
  assert.equal(f.controller.state.active,false);assert.equal(f.controller.state.phase,'conflict');f.setPolicy(false);f.controller.refresh();assert.equal(f.Registry.prototype.pull,foreign);
});
test('unknown registry ABI stays untouched and never claims active',t=>{
  const f=fixture(t);class Unknown{pull(){return [];}ack(){}}
  const chunk={modules:{[id]:(_m,e)=>{e.t=Unknown;}}};f.controller.wrapChunk(chunk);const original=Unknown.prototype.pull;chunk.modules[id]({}, {}, {});
  assert.equal(Unknown.prototype.pull,original);assert.equal(f.controller.state.active,false);assert.equal(f.controller.state.phase,'incompatible');
});
test('new instances and duplicate completion notifications use native bookkeeping',t=>{
  const {Registry}=fixture(t);for(let i=0;i<2;i++){const r=new Registry();r.pending.push({id:'replay'},{id:'replay'});assert.deepEqual(r.pull('chat'),[]);assert.equal(r.inflight.length,0);assert.deepEqual(r.acked,['replay','replay']);}
});
test('cleanup restores only owned hooks and removes runtime heartbeat',t=>{
  const f=fixture(t);assert.equal(JSON.parse(readFileSync(f.options.status)).active,true);f.controller.close();assert.equal(f.module._load,f.originalLoad);assert.equal(f.Registry.prototype.pull,f.originalPull);
});

test('other work kinds are passed through to their native consumer',t=>{
  const {Registry}=fixture(t);const registry=new Registry();registry.pending.push({id:'goal',payload:{kind:'goal'}},{id:'shell'});
  assert.deepEqual(registry.pull('chat'),[{id:'goal',payload:{kind:'goal'}}]);assert.deepEqual(registry.acked,['shell']);
});
test('preload does not leak its bootstrap or require flag to child Node commands',t=>{
  const root=mkdtempSync(join(tmpdir(),'ah-cli-boot-'));t.after(()=>rmSync(root,{recursive:true,force:true}));
  const index=join(root,'index.js'),policy=join(root,'policy.json');writeFileSync(policy,'{"enabled":true}');
  writeFileSync(index,`const cp=require('node:child_process');const r=cp.spawnSync(process.execPath,['-e','process.stdout.write(JSON.stringify({boot:process.env.ASKHUMAN_CURSOR_CLI_BOOT,options:process.env.NODE_OPTIONS}))'],{encoding:'utf8'});process.stdout.write(r.stdout);process.exit(r.status)`);
  const preload=fileURLToPath(new URL('../src-tauri/src/cursor_cli/preload.cjs',import.meta.url));
  const boot=JSON.stringify({index,policy,status:join(root,'status.json'),revision:'test'});
  const result=spawnSync(process.execPath,[index],{encoding:'utf8',env:{...process.env,NODE_OPTIONS:`--require ${JSON.stringify(preload)}`,ASKHUMAN_CURSOR_CLI_BOOT:boot}});
  assert.equal(result.status,0,result.stderr);assert.deepEqual(JSON.parse(result.stdout),{});
});
