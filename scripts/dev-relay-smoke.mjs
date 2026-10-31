// Actual private daemons and a loopback Telegram fixture. No real bot, GUI or model.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import http from "node:http";
import { spawn, spawnSync } from "node:child_process";
import { createConnection } from "node:net";
import { createInterface } from "node:readline";
import { createHash, randomUUID } from "node:crypto";
import { sourceFingerprint, recordBuild } from "./dev-build-record.mjs";
if (process.platform === "win32") { console.log("SKIP: private HOME daemon fixture uses Unix sockets"); process.exit(0); }
const binary = path.resolve(process.argv[2] ?? "src-tauri/target/local-install/AskHuman");
const temp = fs.realpathSync(fs.mkdtempSync(path.join(process.platform === "darwin" ? "/tmp" : os.tmpdir(), "ahr-")));
const home = path.join(temp, "user"), root = path.join(temp, "tree");
fs.mkdirSync(home); fs.mkdirSync(root);
const env = { ...process.env, HOME: home, ASKHUMAN_NO_KEYCHAIN: "1", GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: path.join(temp,"gitconfig") };
fs.writeFileSync(env.GIT_CONFIG_GLOBAL, "");
for (const key of Object.keys(env)) if (key.startsWith("ASKHUMAN_") && key !== "ASKHUMAN_NO_KEYCHAIN") delete env[key];
const formalHome = path.join(home, ".askhuman"), devHome = path.join(root, ".askhuman-dev/home");
const system = path.join(temp, "formal/AskHuman");
fs.mkdirSync(path.dirname(system)); fs.copyFileSync(binary, system); fs.chmodSync(system, 0o700);
const children = [], updates = [];
let updateId = 0, messageId = 0, autoReply = true, failSend = false, cards = 0;
function callback(mid, data) { return { update_id: ++updateId, callback_query: { id: String(updateId), data, from: {id: 1}, message: {message_id: mid, chat: {id: 1}} } }; }
const api = http.createServer(async (req,res) => {
  let raw = ""; for await (const part of req) raw += part;
  const body = raw ? JSON.parse(raw) : {};
  const method = req.url.split("/").at(-1);
  let result = true;
  if (method === "getUpdates") {
    await new Promise(resolve => setTimeout(resolve, 50));
    result = updates.splice(0);
  } else if (method === "sendMessage") {
    if (failSend) { res.end(JSON.stringify({ok:false,description:"fixture delivery failure"})); return; }
    const mid = ++messageId;
    result = { message_id: mid, chat: {id: 1}, text: body.text };
    const buttons = body.reply_markup?.inline_keyboard?.flat() ?? [];
    const confirm = buttons.some(button => button.callback_data?.startsWith("pc:"));
    const ask = buttons.some(button => button.callback_data === "submit");
    if (ask || confirm) {
      cards++;
      if (autoReply) setTimeout(() => {
        updates.push(callback(mid, confirm ? "pc:do:0" : "toggle:0"));
        updates.push(callback(mid, confirm ? "pc:submit" : "submit"));
      }, 150);
    }
  }
  res.setHeader("content-type","application/json"); res.end(JSON.stringify({ok:true,result}));
});
await new Promise(resolve => api.listen(0, "127.0.0.1", resolve));
function run(args, cwd = temp) {
  const out = spawnSync(system,args,{cwd,env,encoding:"utf8",timeout:15000});
  assert.equal(out.status,0,out.stderr); return out.stdout;
}
function write(file,value) { fs.mkdirSync(path.dirname(file),{recursive:true}); fs.writeFileSync(file,JSON.stringify(value),{mode:0o600}); }
function frame(home, message, terminal) {
  return new Promise((resolve,reject) => {
    const socket=createConnection(path.join(home,"daemon.sock")), rows=[];
    const timer=setTimeout(()=>{socket.destroy();reject(new Error("IPC fixture timed out: "+JSON.stringify(rows)));},15000);
    socket.on("error",error=>{clearTimeout(timer);reject(error);});
    createInterface({input:socket}).on("error",()=>{}).on("line",line=>{
      const value=JSON.parse(line);rows.push(value);
      if(terminal(value)) {clearTimeout(timer);socket.destroy();resolve(rows);}
    });
    socket.on("connect",()=>socket.write(JSON.stringify(message)+"\n"));
  });
}
async function status(home) { return (await frame(home,{type:"status"},v=>v.type==="status"))[0]; }
async function until(fn) {
  const end=Date.now()+15000;
  while(Date.now()<end) { try { if(await fn()) return; } catch {} await new Promise(r=>setTimeout(r,50)); }
  throw new Error("fixture condition timed out");
}
function daemon(args) {
  const log=fs.openSync(path.join(temp,"daemon-"+children.length+".log"),"a");
  const child=spawn(system,args,{env,cwd:temp,stdio:["ignore",log,log]}); fs.closeSync(log); children.push(child);
}
async function freshProjectAsk() {
  const log=fs.openSync(path.join(temp,"project-mcp.log"),"a");
  const child=spawn(process.execPath,[path.resolve("scripts/dev.mjs"),"mcp","--worktree",root],{env,cwd:temp,stdio:["pipe","pipe",log]});
  fs.closeSync(log);children.push(child);
  const pending=new Map();let nextId=0;
  createInterface({input:child.stdout}).on("line",line=>{
    const value=JSON.parse(line);pending.get(value.id)?.(value);pending.delete(value.id);
  });
  const call=(method,params)=>new Promise((resolve,reject)=>{
    const id=++nextId,timer=setTimeout(()=>reject(new Error("project MCP call timed out: "+method)),30000);
    pending.set(id,value=>{clearTimeout(timer);resolve(value);});
    child.stdin.write(JSON.stringify({jsonrpc:"2.0",id,method,params})+"\n");
  });
  const initialized=await call("initialize",{protocolVersion:"2025-11-25",capabilities:{},clientInfo:{name:"fresh-verification-regression",version:"1"}});
  assert.ok(initialized.result,JSON.stringify(initialized));
  child.stdin.write(JSON.stringify({jsonrpc:"2.0",method:"notifications/initialized"})+"\n");
  const answer=await call("tools/call",{name:"dev_ask",arguments:{worktree:root,message:"fresh installed-build verification",questions:[{question:"Continue?",options:[{text:"fresh-project-answer",recommended:true}]}]}});
  assert.ok(!answer.result?.isError,JSON.stringify(answer));
  assert.match(JSON.stringify(answer),/fresh-project-answer/);
  assert.match(JSON.stringify(answer),/Dev verification metadata/);
  const records=fs.readdirSync(path.join(devHome,"state/verifications")).filter(name=>name.endsWith(".json"));
  assert.ok(records.some(name=>{
    const record=JSON.parse(fs.readFileSync(path.join(devHome,"state/verifications",name)));
    return record.status==="completed"&&record.stdout?.includes("fresh-project-answer");
  }),"the actual worker must create and finish a new verification record");
  child.stdin.end();
}
let build, target, profile;
function interaction(delivery="remote") {
  const id=randomUUID(), now=Date.now();
  const info={id,worktree:root,build,delivery,relay:false,testProfile:profile.id};
  write(path.join(devHome,"state/verifications",id+".json"),{version:1,info,requestFingerprint:"fixture",status:"pending",executorPid:process.pid,stdout:null,exitCode:null,createdAtMs:now,updatedAtMs:now});
  write(path.join(formalHome,"state/development-relay/relay-grants",id+".json"),{version:1,info,expiresAtMs:now+60000,requestFingerprint:null});
  return info;
}
function askTask(info, stop=false) {
  return {type:"submit",message:{text:"isolated relay fixture",files:[],development:info},
    questions:[{message:"Continue?",predefinedOptions:[{text:"fixture work",recommended:true,todoId:stop?"dev-todo":null}]}],
    isMarkdown:false,source:"Dev fixture",lang:"en",project:root,callerCwd:root,selectOnly:false,single:false,outputFormat:"json",
    recordHistory:false,stopConfirmation:stop,agentKind:"codex",agentSessionId:"test-session",mcpInstanceId:"fixture",agentPid:null,callerPid:0,fromMcp:true,perfId:"",perfAutodismiss:false,whatsNext:false};
}
try {
  spawnSync("git",["init","--quiet",root],{env});
  run(["dev","enable","--agent","pi"],root);
  const devBinary=path.join(root,".askhuman-dev/bin/AskHuman");
  fs.copyFileSync(binary,devBinary); fs.chmodSync(devBinary,0o700);
  build=recordBuild(root,devBinary,sourceFingerprint(root));
  target=JSON.parse(run(["--instance","dev","--worktree",root,"dev","target"]));
  write(path.join(formalHome,"config.json"),{general:{popupPrewarm:false,menuBarIcon:"off"},channels:{autoActivation:true,popup:{enabled:false},telegram:{enabled:true,botToken:"123:fixture",chatId:"1",apiBaseUrl:"http://127.0.0.1:"+api.address().port}}});
  write(path.join(formalHome,"state/auto-channel.json"),{channel:"popup",updated_at:1});
  write(path.join(devHome,"config.json"),{general:{popupPrewarm:false,menuBarIcon:"off"},channels:{popup:{enabled:false}}});
  write(path.join(formalHome,"state/todos.json"),{projects:{[root]:[{id:"dev-todo",text:"formal protected",auto:false,createdAt:1}]}});
  write(path.join(devHome,"state/todos.json"),{projects:{[root]:[{id:"dev-todo",text:"private work",auto:false,createdAt:1}]}});
  const directory=path.join(root,".askhuman-dev/clients/codex"), neutral=path.join(temp,"neutral");
  fs.mkdirSync(directory,{recursive:true});fs.mkdirSync(path.join(neutral,".git"),{recursive:true});
  profile={version:1,id:randomUUID(),kind:"codex",worktree:root,directory,neutralWorkspace:neutral,executable:"fixture",build,prepared:true};
  write(path.join(neutral,"askhuman-owner.json"),{profile:profile.id,worktree:root});
  write(path.join(devHome,"state/test-profiles",profile.id+".json"),profile);
  const binding=createHash("sha256").update("codex\0test-session").digest("hex");
  write(path.join(devHome,"state/test-session-bindings",binding+".json"),{profile:profile.id,kind:"codex",sessionId:"test-session",transcript:null});
  daemon(["daemon","run"]);
  daemon(["--instance","dev","--worktree",root,"daemon","run"]);
  await until(async()=> (await status(formalHome)).info?.developmentVerificationVersion===2 || (await status(formalHome)).developmentVerificationVersion===2);
  await until(async()=> !!(await status(devHome)));
  const ordinaryBefore=fs.readFileSync(path.join(formalHome,"state/todos.json"));
  await freshProjectAsk();
  const answer=await frame(devHome,askTask(interaction()),v=>v.type==="final");
  assert.equal(answer.at(-1).exit_code,0,JSON.stringify(answer));
  assert.match(answer.at(-1).stdout,/fixture work/);
  const stop=await frame(devHome,askTask(interaction(),true),v=>v.type==="final");
  assert.equal(stop.at(-1).exit_code,0,JSON.stringify(stop));
  assert.deepEqual(fs.readFileSync(path.join(formalHome,"state/todos.json")),ordinaryBefore);
  assert.equal(JSON.parse(fs.readFileSync(path.join(devHome,"state/todos.json"))).projects[root]?.length ?? 0,0);
  const spec={title:"Dev permission",context:["agent","project","workspace","tool","permission_mode","created_at"].map(id=>({id,label:id,value:"fixture",kind:"text"})),
    detail:{summary:"fixture permission",bodyMd:""},choices:[{id:"remember_test",label:"Remember in Dev",description:"",role:"primary"},{id:"deny",label:"Deny",description:"",role:"destructive"}],
    presentation:{type:"singleSelectSubmit",input:null,submit_label:"Submit",default_action_id:null},dismissActionId:"deny"};
  const confirmation={type:"submitConfirm",development:interaction(),callerCwd:root,spec,popupEdit:null,source:"Dev fixture",lang:"en",project:root,agentKind:"codex",agentSessionId:"test-session",callerPid:0,
    memory:{saves:[{actionId:"remember_test",namespace:"session",rules:[{kind:"mcpTool",tool:"mcp__fixture__tool"}]}]}};
  const decision=await frame(devHome,confirmation,v=>v.type==="confirmFinal"||v.type==="confirmFallback");
  assert.equal(decision.at(-1).type,"confirmFinal",JSON.stringify(decision));
  assert.equal(decision.at(-1).result.actionId,"remember_test");
  assert.match(fs.readFileSync(path.join(devHome,"state/codex_rules.json"),"utf8"),/test-session/);
  assert.equal(fs.existsSync(path.join(formalHome,"state/codex_rules.json")),false);
  assert.equal(JSON.parse(fs.readFileSync(path.join(formalHome,"state/auto-channel.json"))).channel,"popup");
  autoReply=false;
  const count=cards;
  const cancelled=createConnection(path.join(devHome,"daemon.sock"));
  cancelled.on("error",()=>{}); cancelled.on("connect",()=>cancelled.write(JSON.stringify(askTask(interaction()))+"\n"));cancelled.resume();
  await until(()=>cards>count);cancelled.destroy();
  await until(async()=> {
    const values=JSON.stringify(await status(formalHome));
    return /"activeRequests":0/.test(values);
  });
  failSend=true;
  const failed=await frame(devHome,askTask(interaction()),v=>v.type==="final");
  assert.notEqual(failed.at(-1).exit_code,0,"send failure cannot become local success");
  console.log("PASS: fresh project MCP ask and retained result; actual relay asks, native Stop/todo ownership, permission memory ownership, daily active slot preserved, cancellation and send failure");
} catch(error) {
  for(const name of fs.readdirSync(temp).filter(name=>name.endsWith(".log"))) console.error(fs.readFileSync(path.join(temp,name),"utf8").slice(-5000));
  throw error;
} finally {
  for(const child of children) child.kill();
  await Promise.all(children.map(child=>new Promise(resolve=>{if(child.exitCode!==null)return resolve();child.once("exit",resolve);setTimeout(()=>{child.kill("SIGKILL");resolve();},2000).unref();})));
  await new Promise(resolve=>api.close(resolve));fs.rmSync(temp,{recursive:true,force:true});
}
